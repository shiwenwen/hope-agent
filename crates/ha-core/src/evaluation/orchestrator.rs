use super::artifact_store::EvalArtifactStore;
use super::store::EvalRepository;
use super::types::{
    EvalExperimentStatus, EvalPreview, EvalReadiness, EvalResolvedLaunch, EvalTrialRecord,
    EvalWorkerEvent, EVALUATION_EVENT,
};
use crate::event_bus::EventBus;
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use ha_eval_spec::app::{AppDebugRetention, EvalAppPlan, EvalAppProfile};
use ha_eval_spec::model::{validate_evidence_shape, ModelCampaignEvidence};
use std::path::Component;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

#[async_trait]
pub trait EvalWorkerRuntime: Send + Sync + 'static {
    async fn readiness(&self) -> Result<EvalReadiness>;
    async fn list_profiles(&self) -> Result<Vec<EvalAppProfile>>;
    async fn list_suites(&self) -> Result<Vec<ha_eval_spec::app::AppEvalSuiteCatalog>>;
    async fn preview(&self, launch: &EvalResolvedLaunch) -> Result<EvalAppPlan>;
    async fn start(
        &self,
        run_id: &str,
        plan: &EvalAppPlan,
        launch: EvalResolvedLaunch,
    ) -> Result<mpsc::UnboundedReceiver<EvalWorkerEvent>>;
    async fn cancel(&self, run_id: &str) -> Result<()>;
    async fn cleanup(&self, run_id: &str) -> Result<()>;
}

pub struct EvalOrchestrator<R: EvalWorkerRuntime> {
    repository: EvalRepository,
    artifacts: EvalArtifactStore,
    runtime: Arc<R>,
    events: Arc<dyn EventBus>,
    active: Arc<Mutex<Option<String>>>,
}

impl<R: EvalWorkerRuntime> Clone for EvalOrchestrator<R> {
    fn clone(&self) -> Self {
        Self {
            repository: self.repository.clone(),
            artifacts: self.artifacts.clone(),
            runtime: Arc::clone(&self.runtime),
            events: Arc::clone(&self.events),
            active: Arc::clone(&self.active),
        }
    }
}

impl<R: EvalWorkerRuntime> EvalOrchestrator<R> {
    pub fn new(
        repository: EvalRepository,
        artifacts: EvalArtifactStore,
        runtime: Arc<R>,
        events: Arc<dyn EventBus>,
    ) -> Result<Self> {
        repository.reconcile_interrupted()?;
        artifacts.prune_expired(&repository)?;
        Ok(Self {
            repository,
            artifacts,
            runtime,
            events,
            active: Arc::new(Mutex::new(None)),
        })
    }

    pub fn repository(&self) -> &EvalRepository {
        &self.repository
    }

    pub fn runtime(&self) -> Arc<R> {
        Arc::clone(&self.runtime)
    }

    pub async fn readiness(&self) -> Result<EvalReadiness> {
        self.runtime.readiness().await
    }

    pub async fn list_profiles(&self) -> Result<Vec<EvalAppProfile>> {
        self.runtime.list_profiles().await
    }

    pub async fn preview(&self, launch: &EvalResolvedLaunch) -> Result<EvalPreview> {
        let plan = self.runtime.preview(launch).await?;
        let estimated_trials = plan
            .campaigns
            .iter()
            .map(|campaign| campaign.resolved_plan.trials.len())
            .sum();
        Ok(EvalPreview {
            max_cost_usd: plan.campaign_budget.max_cost_usd,
            max_wall_seconds: plan.campaign_budget.max_wall_seconds,
            plan,
            estimated_trials,
        })
    }

    pub async fn start(
        &self,
        launch: EvalResolvedLaunch,
        parent_experiment_id: Option<&str>,
        expected_plan_digest: Option<&str>,
    ) -> Result<String> {
        let readiness = self.runtime.readiness().await?;
        if !readiness.can_run {
            bail!(
                "evaluation runtime is not ready: {}",
                readiness.issues.join(", ")
            );
        }
        let mut active = self.active.lock().await;
        if let Some(active) = active.as_ref() {
            bail!("evaluation experiment {active} is already running");
        }
        if let Some(parent) = parent_experiment_id {
            let parent = self
                .repository
                .get_experiment(parent)?
                .ok_or_else(|| anyhow!("parent evaluation experiment not found"))?;
            if !parent.status.is_terminal() {
                bail!("only a terminal evaluation experiment may be retried");
            }
        }
        let plan = self.runtime.preview(&launch).await?;
        if expected_plan_digest.is_some_and(|expected| expected != plan.plan_digest) {
            bail!("evaluation preview is stale; regenerate it before starting");
        }
        let run_id = format!("evalrun-{}", uuid::Uuid::new_v4());
        let redacted_request = serde_json::to_value(launch.request.redacted())?;
        self.repository.create_experiment(
            &run_id,
            &plan,
            &redacted_request,
            parent_experiment_id,
        )?;
        self.repository
            .transition(&run_id, EvalExperimentStatus::Planning, None)?;
        let receiver = match self.runtime.start(&run_id, &plan, launch).await {
            Ok(receiver) => receiver,
            Err(error) => {
                let cleanup_error = self.runtime.cleanup(&run_id).await.err();
                self.repository.transition(
                    &run_id,
                    EvalExperimentStatus::Failed,
                    Some(&match cleanup_error {
                        Some(cleanup) => format!(
                            "{}; cleanup failed: {}",
                            safe_error(&error),
                            safe_error(&cleanup)
                        ),
                        None => safe_error(&error),
                    }),
                )?;
                return Err(error);
            }
        };
        self.repository
            .transition(&run_id, EvalExperimentStatus::Running, None)?;
        *active = Some(run_id.clone());
        drop(active);
        self.emit(&run_id, "started");
        self.spawn_event_consumer(run_id.clone(), receiver);
        Ok(run_id)
    }

    pub async fn cancel(&self, run_id: &str) -> Result<()> {
        let active = self.active.lock().await;
        if active.as_deref() != Some(run_id) {
            let record = self
                .repository
                .get_experiment(run_id)?
                .ok_or_else(|| anyhow!("evaluation experiment not found"))?;
            if record.status.is_terminal() {
                return Ok(());
            }
            bail!("evaluation experiment is not active in this process");
        }
        self.repository
            .transition(run_id, EvalExperimentStatus::Cancelling, None)?;
        drop(active);
        if let Err(error) = self.runtime.cancel(run_id).await {
            self.repository.transition(
                run_id,
                EvalExperimentStatus::Failed,
                Some(&safe_error(&error)),
            )?;
            self.clear_active(run_id).await;
            return Err(error);
        }
        self.emit(run_id, "cancelling");
        Ok(())
    }

    fn spawn_event_consumer(
        &self,
        run_id: String,
        mut receiver: mpsc::UnboundedReceiver<EvalWorkerEvent>,
    ) {
        let orchestrator = self.clone();
        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                let terminal = match orchestrator.handle_event(&run_id, event).await {
                    Ok(terminal) => terminal,
                    Err(error) => {
                        let _ = orchestrator.repository.transition(
                            &run_id,
                            EvalExperimentStatus::Failed,
                            Some(&safe_error(&error)),
                        );
                        orchestrator.emit(&run_id, "failed");
                        true
                    }
                };
                if terminal {
                    orchestrator.clear_active(&run_id).await;
                    return;
                }
            }
            let current = orchestrator
                .repository
                .get_experiment(&run_id)
                .ok()
                .flatten();
            if current.is_some_and(|record| !record.status.is_terminal()) {
                let cleanup_error = orchestrator.runtime.cleanup(&run_id).await.err();
                let message = cleanup_error
                    .as_ref()
                    .map(|error| {
                        format!(
                            "Evaluation worker event stream closed unexpectedly; cleanup failed: {}",
                            safe_error(error)
                        )
                    })
                    .unwrap_or_else(|| {
                        "Evaluation worker event stream closed unexpectedly".to_string()
                    });
                let _ = orchestrator.repository.transition(
                    &run_id,
                    EvalExperimentStatus::Interrupted,
                    Some(&message),
                );
                orchestrator.emit(&run_id, "interrupted");
            }
            orchestrator.clear_active(&run_id).await;
        });
    }

    async fn handle_event(&self, run_id: &str, event: EvalWorkerEvent) -> Result<bool> {
        match event {
            EvalWorkerEvent::Phase {
                experiment_id: _,
                phase,
                completed,
                total,
            } => {
                self.events.emit(
                    EVALUATION_EVENT,
                    serde_json::json!({
                        "experimentId": run_id,
                        "change": "progress",
                        "phase": phase,
                        "completed": completed,
                        "total": total,
                    }),
                );
                Ok(false)
            }
            EvalWorkerEvent::TrialStarted {
                campaign_id,
                trial_id,
                completed,
                total,
                ..
            } => {
                self.repository
                    .mark_campaign_running(run_id, &campaign_id)?;
                self.events.emit(
                    EVALUATION_EVENT,
                    serde_json::json!({
                        "experimentId": run_id,
                        "change": "trial_started",
                        "campaignId": campaign_id,
                        "trialId": trial_id,
                        "completed": completed,
                        "total": total,
                    }),
                );
                Ok(false)
            }
            EvalWorkerEvent::TrialCompleted {
                campaign_id,
                trial_id,
                completed,
                total,
                outcome,
                wall_ms,
                input_tokens,
                output_tokens,
                cost_usd,
                model_calls,
                tool_calls,
                suite_id,
                case_id,
                arm,
                attempt,
                failure_class,
                ..
            } => {
                self.repository.record_trial_progress(
                    run_id,
                    &EvalTrialRecord {
                        id: trial_id.clone(),
                        campaign_id: campaign_id.clone(),
                        suite_id,
                        case_id,
                        arm,
                        outcome,
                        attempt,
                        duration_ms: wall_ms,
                        model_calls: u32::try_from(model_calls).map_err(|_| {
                            anyhow!("trial model-call count exceeds storage limits")
                        })?,
                        tool_calls,
                        input_tokens,
                        output_tokens,
                        cost_usd,
                        trace_artifact_sha256: None,
                        failure_class,
                    },
                )?;
                self.repository.update_progress(run_id, completed, total)?;
                self.events.emit(
                    EVALUATION_EVENT,
                    serde_json::json!({
                        "experimentId": run_id,
                        "change": "trial_completed",
                        "campaignId": campaign_id,
                        "trialId": trial_id,
                        "completed": completed,
                        "total": total,
                        "outcome": outcome,
                        "wallMs": wall_ms,
                        "inputTokens": input_tokens,
                        "outputTokens": output_tokens,
                        "costUsd": cost_usd,
                        "modelCalls": model_calls,
                        "toolCalls": tool_calls,
                    }),
                );
                Ok(false)
            }
            EvalWorkerEvent::BudgetWarning {
                dimension,
                observed,
                limit,
                ratio,
                ..
            } => {
                self.events.emit(
                    EVALUATION_EVENT,
                    serde_json::json!({
                        "experimentId": run_id,
                        "change": "budget_warning",
                        "dimension": dimension,
                        "observed": observed,
                        "limit": limit,
                        "ratio": ratio,
                    }),
                );
                Ok(false)
            }
            EvalWorkerEvent::ArtifactWritten {
                campaign_id,
                path,
                sha256,
                ..
            } => {
                self.events.emit(
                    EVALUATION_EVENT,
                    serde_json::json!({
                        "experimentId": run_id,
                        "change": "artifact_written",
                        "campaignId": campaign_id,
                        "path": path,
                        "sha256": sha256,
                    }),
                );
                Ok(false)
            }
            EvalWorkerEvent::Evidence {
                experiment_id: _,
                campaign_id,
                evidence_path,
            } => {
                let path = Path::new(&evidence_path);
                let artifact = self.artifacts.put_file(path, 256 * 1024 * 1024)?;
                let evidence: ModelCampaignEvidence =
                    serde_json::from_slice(&std::fs::read(path)?)?;
                validate_evidence_shape(&evidence)?;
                if evidence.campaign_id != campaign_id
                    || evidence.source != ha_eval_spec::model::ModelCampaignSource::LocalApp
                    || evidence.execution_profile.is_none()
                {
                    bail!("local worker produced evidence with an invalid source or identity");
                }
                self.repository
                    .index_evidence(run_id, &evidence, &artifact)?;
                self.index_local_campaign_artifacts(run_id, path, &evidence)?;
                self.emit(run_id, "campaign_completed");
                Ok(false)
            }
            EvalWorkerEvent::Completed { .. } => {
                let detail = self
                    .repository
                    .detail(run_id)?
                    .ok_or_else(|| anyhow!("evaluation experiment disappeared"))?;
                if detail.campaigns.is_empty()
                    || detail
                        .campaigns
                        .iter()
                        .any(|campaign| campaign.evidence_artifact_sha256.is_none())
                {
                    bail!("worker completed before all child evidence was indexed");
                }
                self.runtime.cleanup(run_id).await?;
                self.repository
                    .transition(run_id, EvalExperimentStatus::Completed, None)?;
                self.emit(run_id, "completed");
                Ok(true)
            }
            EvalWorkerEvent::Cancelled { .. } => {
                let current = self
                    .repository
                    .get_experiment(run_id)?
                    .ok_or_else(|| anyhow!("evaluation experiment disappeared"))?;
                if current.status != EvalExperimentStatus::Cancelling {
                    bail!("worker emitted cancelled without a user cancellation request");
                }
                self.runtime.cleanup(run_id).await?;
                self.repository
                    .transition(run_id, EvalExperimentStatus::Cancelled, None)?;
                self.emit(run_id, "cancelled");
                Ok(true)
            }
            EvalWorkerEvent::Failed { code, message, .. } => {
                let error = match self.runtime.cleanup(run_id).await {
                    Ok(()) => format!("{code}: {message}"),
                    Err(cleanup) => {
                        format!(
                            "{code}: {message}; cleanup failed: {}",
                            safe_error(&cleanup)
                        )
                    }
                };
                self.repository
                    .transition(run_id, EvalExperimentStatus::Failed, Some(&error))?;
                self.emit(run_id, "failed");
                Ok(true)
            }
        }
    }

    fn emit(&self, run_id: &str, change: &str) {
        self.events.emit(
            EVALUATION_EVENT,
            serde_json::json!({"experimentId": run_id, "change": change}),
        );
    }

    fn index_local_campaign_artifacts(
        &self,
        run_id: &str,
        evidence_path: &Path,
        evidence: &ModelCampaignEvidence,
    ) -> Result<()> {
        let retention = self.repository.experiment_debug_retention(run_id)?;
        let parent = evidence_path
            .parent()
            .ok_or_else(|| anyhow!("evaluation evidence has no parent directory"))?
            .canonicalize()?;
        let summary = parent.join("summary.md");
        if summary.is_file()
            && !std::fs::symlink_metadata(&summary)?
                .file_type()
                .is_symlink()
        {
            let artifact = self.artifacts.put_file(&summary, 16 * 1024 * 1024)?;
            self.repository.index_run_artifact(
                run_id,
                &evidence.campaign_id,
                "summary",
                &artifact,
                90,
            )?;
        }
        let days = match retention {
            AppDebugRetention::MetricsOnly => return Ok(()),
            AppDebugRetention::Redacted => 30,
            AppDebugRetention::FullLocal => 7,
        };
        for declared in &evidence.artifacts {
            let relative = Path::new(&declared.path);
            if relative.is_absolute()
                || !relative
                    .components()
                    .all(|component| matches!(component, Component::Normal(_)))
            {
                bail!("evaluation evidence declares an unsafe artifact path");
            }
            let path = parent.join(relative);
            let metadata = std::fs::symlink_metadata(&path)?;
            let canonical = path.canonicalize()?;
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || !canonical.starts_with(&parent)
                || ha_eval_spec::digest_file(&canonical)? != declared.sha256
            {
                bail!("evaluation evidence artifact failed containment or digest checks");
            }
            let artifact = self.artifacts.put_file(&canonical, 256 * 1024 * 1024)?;
            let kind = if declared.path.starts_with("traces/") {
                "redacted_trace"
            } else {
                "redacted_shard"
            };
            self.repository.index_run_artifact(
                run_id,
                &evidence.campaign_id,
                kind,
                &artifact,
                days,
            )?;
        }
        Ok(())
    }

    async fn clear_active(&self, run_id: &str) {
        let mut active = self.active.lock().await;
        if active.as_deref() == Some(run_id) {
            *active = None;
        }
    }
}

fn safe_error(error: &anyhow::Error) -> String {
    error.to_string().chars().take(2_000).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_bus::BroadcastEventBus;

    struct UnavailableRuntime;

    #[async_trait]
    impl EvalWorkerRuntime for UnavailableRuntime {
        async fn readiness(&self) -> Result<EvalReadiness> {
            Ok(EvalReadiness {
                available: false,
                can_run: false,
                remote_run_enabled: false,
                signed_import_available: false,
                hello: None,
                issues: vec!["sidecar_missing".into()],
                signed_import_issues: vec!["trust_registry_missing".into()],
            })
        }
        async fn list_profiles(&self) -> Result<Vec<EvalAppProfile>> {
            Ok(Vec::new())
        }
        async fn list_suites(&self) -> Result<Vec<ha_eval_spec::app::AppEvalSuiteCatalog>> {
            Ok(Vec::new())
        }
        async fn preview(&self, _launch: &EvalResolvedLaunch) -> Result<EvalAppPlan> {
            bail!("unavailable")
        }
        async fn start(
            &self,
            _run_id: &str,
            _plan: &EvalAppPlan,
            _launch: EvalResolvedLaunch,
        ) -> Result<mpsc::UnboundedReceiver<EvalWorkerEvent>> {
            bail!("unavailable")
        }
        async fn cancel(&self, _run_id: &str) -> Result<()> {
            Ok(())
        }
        async fn cleanup(&self, _run_id: &str) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn unavailable_runtime_fails_before_creating_history() {
        let temp = tempfile::tempdir().unwrap();
        let repository = EvalRepository::open(temp.path().join("evals.db")).unwrap();
        let orchestrator = EvalOrchestrator::new(
            repository.clone(),
            EvalArtifactStore::open(temp.path().join("artifacts")).unwrap(),
            Arc::new(UnavailableRuntime),
            Arc::new(BroadcastEventBus::new(4)),
        )
        .unwrap();
        assert!(!orchestrator.readiness().await.unwrap().can_run);
        assert!(repository
            .list_experiments(&Default::default())
            .unwrap()
            .is_empty());
    }
}
