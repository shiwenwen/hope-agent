use super::artifact_store::StoredEvalArtifact;
use super::types::{
    evidence_summary, EvalAnnotationRecord, EvalBaselineRecord, EvalCampaignRecord,
    EvalExperimentDetail, EvalExperimentRecord, EvalExperimentStatus, EvalHistoryKind,
    EvalHistoryQuery, EvalImportResult, EvalIntegrity, EvalTrialRecord,
};
use anyhow::{anyhow, bail, Result};
use chrono::{Duration, SecondsFormat, Utc};
use ha_eval_spec::app::{
    evidence_trust_key_fingerprint, AppDebugRetention, EvalAppPlan, EvalAppRunRequest,
    EvidenceKeyStatus, EvidenceTrustRegistry,
};
use ha_eval_spec::model::{
    ModelCampaignEvidence, ModelCampaignOutcome, ModelCampaignSource, ModelCampaignTier,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: i64 = 5;

#[derive(Debug, Clone)]
pub struct EvalRepository {
    path: PathBuf,
}

impl EvalRepository {
    pub fn open(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let repository = Self { path };
        repository.migrate()?;
        Ok(repository)
    }

    pub fn default_repository() -> Result<Self> {
        Self::open(crate::paths::evals_db_path()?)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn connection(&self) -> Result<Connection> {
        let connection = Connection::open(&self.path)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(connection)
    }

    fn migrate(&self) -> Result<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS eval_schema_meta (
              singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
              version INTEGER NOT NULL
            );
            INSERT OR IGNORE INTO eval_schema_meta(singleton, version) VALUES(1, 0);

            CREATE TABLE IF NOT EXISTS eval_experiments (
              id TEXT PRIMARY KEY,
              kind TEXT NOT NULL,
              profile_id TEXT NOT NULL,
              source TEXT NOT NULL,
              integrity TEXT NOT NULL,
              status TEXT NOT NULL,
              git_ref TEXT NOT NULL,
              dirty INTEGER NOT NULL,
              app_version TEXT NOT NULL,
              plan_digest TEXT,
              parent_experiment_id TEXT REFERENCES eval_experiments(id),
              request_json TEXT,
              runtime_json TEXT,
              created_at TEXT NOT NULL,
              started_at TEXT,
              completed_at TEXT,
              total_trials INTEGER NOT NULL DEFAULT 0,
              completed_trials INTEGER NOT NULL DEFAULT 0,
              passed_trials INTEGER NOT NULL DEFAULT 0,
              failed_trials INTEGER NOT NULL DEFAULT 0,
              infra_error_trials INTEGER NOT NULL DEFAULT 0,
              max_cost_usd REAL,
              observed_cost_usd REAL,
              pinned INTEGER NOT NULL DEFAULT 0,
              error TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_eval_experiments_created
              ON eval_experiments(created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_eval_experiments_status
              ON eval_experiments(status, created_at DESC);

            CREATE TABLE IF NOT EXISTS eval_campaigns (
              experiment_id TEXT NOT NULL REFERENCES eval_experiments(id) ON DELETE CASCADE,
              id TEXT NOT NULL,
              model_digest TEXT NOT NULL,
              provider_config_digest TEXT NOT NULL,
              status TEXT NOT NULL,
              evidence_artifact_sha256 TEXT,
              aggregate_status TEXT,
              total_trials INTEGER NOT NULL DEFAULT 0,
              passed_trials INTEGER NOT NULL DEFAULT 0,
              failed_trials INTEGER NOT NULL DEFAULT 0,
              infra_error_trials INTEGER NOT NULL DEFAULT 0,
              duration_ms INTEGER,
              cost_usd REAL,
              PRIMARY KEY(experiment_id, id)
            );

            CREATE TABLE IF NOT EXISTS eval_trials (
              experiment_id TEXT NOT NULL,
              campaign_id TEXT NOT NULL,
              id TEXT NOT NULL,
              suite_id TEXT NOT NULL,
              case_id TEXT NOT NULL,
              arm TEXT NOT NULL,
              outcome TEXT NOT NULL,
              attempt INTEGER NOT NULL,
              duration_ms INTEGER NOT NULL,
              model_calls INTEGER NOT NULL,
              tool_calls INTEGER NOT NULL,
              input_tokens INTEGER,
              output_tokens INTEGER,
              cost_usd REAL,
              trace_artifact_sha256 TEXT,
              failure_class TEXT,
              PRIMARY KEY(experiment_id, campaign_id, id),
              FOREIGN KEY(experiment_id, campaign_id)
                REFERENCES eval_campaigns(experiment_id, id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_eval_trials_case
              ON eval_trials(suite_id, case_id, arm);

            CREATE TABLE IF NOT EXISTS eval_artifacts (
              sha256 TEXT PRIMARY KEY,
              size_bytes INTEGER NOT NULL,
              media_type TEXT NOT NULL,
              created_at TEXT NOT NULL,
              pinned INTEGER NOT NULL DEFAULT 0,
              protected INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS eval_artifact_refs (
              owner_kind TEXT NOT NULL,
              owner_id TEXT NOT NULL,
              artifact_kind TEXT NOT NULL,
              sha256 TEXT NOT NULL REFERENCES eval_artifacts(sha256) ON DELETE CASCADE,
              retention_until TEXT,
              PRIMARY KEY(owner_kind, owner_id, artifact_kind, sha256)
            );
            CREATE INDEX IF NOT EXISTS idx_eval_artifact_refs_retention
              ON eval_artifact_refs(retention_until, sha256);

            CREATE TABLE IF NOT EXISTS eval_imports (
              id TEXT PRIMARY KEY,
              bundle_sha256 TEXT NOT NULL UNIQUE,
              experiment_id TEXT NOT NULL REFERENCES eval_experiments(id),
              evidence_sha256 TEXT NOT NULL,
              integrity TEXT NOT NULL,
              key_id TEXT,
              key_fingerprint TEXT,
              signature_status TEXT NOT NULL,
              imported_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS eval_baselines (
              id TEXT PRIMARY KEY,
              experiment_id TEXT NOT NULL REFERENCES eval_experiments(id),
              tier TEXT NOT NULL,
              approved_by TEXT NOT NULL,
              approved_at TEXT NOT NULL,
              note TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_eval_baselines_tier
              ON eval_baselines(tier, approved_at DESC);

            CREATE TABLE IF NOT EXISTS eval_annotations (
              id TEXT PRIMARY KEY,
              experiment_id TEXT NOT NULL REFERENCES eval_experiments(id),
              campaign_id TEXT,
              trial_id TEXT,
              text TEXT NOT NULL,
              created_at TEXT NOT NULL
            );
            ",
        )?;
        let current: i64 = transaction.query_row(
            "SELECT version FROM eval_schema_meta WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        if current > SCHEMA_VERSION {
            bail!("evals.db schema is newer than this application");
        }
        if current < 3 {
            let has_pinned = {
                let mut statement = transaction.prepare("PRAGMA table_info(eval_experiments)")?;
                let columns = statement
                    .query_map([], |row| row.get::<_, String>(1))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                columns.iter().any(|column| column == "pinned")
            };
            if !has_pinned {
                transaction.execute(
                    "ALTER TABLE eval_experiments ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0",
                    [],
                )?;
            }
        }
        if current < 4 {
            let has_approved_by = {
                let mut statement = transaction.prepare("PRAGMA table_info(eval_baselines)")?;
                let columns = statement
                    .query_map([], |row| row.get::<_, String>(1))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                columns.iter().any(|column| column == "approved_by")
            };
            if !has_approved_by {
                transaction.execute(
                    "ALTER TABLE eval_baselines ADD COLUMN approved_by TEXT NOT NULL DEFAULT 'legacy_local_owner'",
                    [],
                )?;
            }
        }
        if current < 5 {
            let has_key_fingerprint = {
                let mut statement = transaction.prepare("PRAGMA table_info(eval_imports)")?;
                let columns = statement
                    .query_map([], |row| row.get::<_, String>(1))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                columns.iter().any(|column| column == "key_fingerprint")
            };
            if !has_key_fingerprint {
                transaction.execute(
                    "ALTER TABLE eval_imports ADD COLUMN key_fingerprint TEXT",
                    [],
                )?;
            }
        }
        transaction.execute(
            "UPDATE eval_schema_meta SET version=?1 WHERE singleton=1",
            [SCHEMA_VERSION],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn create_experiment(
        &self,
        run_id: &str,
        plan: &EvalAppPlan,
        redacted_request: &serde_json::Value,
        parent_experiment_id: Option<&str>,
    ) -> Result<()> {
        validate_run_id(run_id)?;
        let now = now();
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO eval_experiments(
               id, kind, profile_id, source, integrity, status, git_ref, dirty,
               app_version, plan_digest, parent_experiment_id, request_json,
               runtime_json, created_at, total_trials, max_cost_usd
             ) VALUES(?1,'hope_core',?2,'local_app','local_diagnostic','queued',
               ?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                run_id,
                plan.profile_id,
                plan.reference,
                plan.dirty,
                plan.app_version,
                plan.plan_digest,
                parent_experiment_id,
                serde_json::to_string(redacted_request)?,
                serde_json::to_string(&plan.runtime_environment)?,
                now,
                i64::try_from(
                    plan.campaigns
                        .iter()
                        .map(|campaign| campaign.resolved_plan.trials.len())
                        .sum::<usize>()
                )?,
                plan.campaign_budget.max_cost_usd,
            ],
        )?;
        for campaign in &plan.campaigns {
            connection.execute(
                "INSERT INTO eval_campaigns(
                   experiment_id,id,model_digest,provider_config_digest,status,total_trials
                 ) VALUES(?1,?2,?3,?4,'queued',?5)",
                params![
                    run_id,
                    campaign.campaign_id,
                    campaign.model_digest,
                    campaign.provider_config_digest,
                    i64::try_from(campaign.resolved_plan.trials.len())?,
                ],
            )?;
        }
        Ok(())
    }

    pub fn transition(
        &self,
        run_id: &str,
        next: EvalExperimentStatus,
        error: Option<&str>,
    ) -> Result<()> {
        let current = self
            .get_experiment(run_id)?
            .ok_or_else(|| anyhow!("evaluation experiment not found"))?;
        if !valid_transition(current.status, next) {
            bail!(
                "invalid evaluation transition {} -> {}",
                current.status.as_str(),
                next.as_str()
            );
        }
        let now = now();
        let started = matches!(
            next,
            EvalExperimentStatus::Planning | EvalExperimentStatus::Running
        );
        let completed = next.is_terminal();
        let connection = self.connection()?;
        connection.execute(
            "UPDATE eval_experiments SET status=?2,
               started_at=CASE WHEN ?3 THEN COALESCE(started_at,?5) ELSE started_at END,
               completed_at=CASE WHEN ?4 THEN ?5 ELSE completed_at END,
               error=?6 WHERE id=?1",
            params![run_id, next.as_str(), started, completed, now, error],
        )?;
        if matches!(
            next,
            EvalExperimentStatus::Failed
                | EvalExperimentStatus::Cancelled
                | EvalExperimentStatus::Interrupted
        ) {
            connection.execute(
                "UPDATE eval_campaigns SET status=?2
                 WHERE experiment_id=?1 AND status!='completed'",
                params![run_id, next.as_str()],
            )?;
        }
        Ok(())
    }

    pub fn update_progress(&self, run_id: &str, completed: u32, total: u32) -> Result<()> {
        self.connection()?.execute(
            "UPDATE eval_experiments SET completed_trials=MIN(?2,total_trials),
               total_trials=MAX(total_trials,?3) WHERE id=?1",
            params![run_id, i64::from(completed), i64::from(total)],
        )?;
        Ok(())
    }

    pub fn mark_campaign_running(&self, run_id: &str, campaign_id: &str) -> Result<()> {
        self.connection()?.execute(
            "UPDATE eval_campaigns SET status='running'
             WHERE experiment_id=?1 AND id=?2 AND status='queued'",
            params![run_id, campaign_id],
        )?;
        Ok(())
    }

    /// Persist a completed trial before final campaign evidence exists. This
    /// keeps timeout/cancellation diagnostics authoritative after an App
    /// reload; successful aggregation later replaces these rows from the
    /// verified evidence artifact.
    pub fn record_trial_progress(&self, run_id: &str, trial: &EvalTrialRecord) -> Result<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO eval_trials(
               experiment_id,campaign_id,id,suite_id,case_id,arm,outcome,attempt,
               duration_ms,model_calls,tool_calls,input_tokens,output_tokens,cost_usd,
               trace_artifact_sha256,failure_class
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,NULL,?15)
             ON CONFLICT(experiment_id,campaign_id,id) DO UPDATE SET
               suite_id=excluded.suite_id,case_id=excluded.case_id,arm=excluded.arm,
               outcome=excluded.outcome,attempt=excluded.attempt,
               duration_ms=excluded.duration_ms,model_calls=excluded.model_calls,
               tool_calls=excluded.tool_calls,input_tokens=excluded.input_tokens,
               output_tokens=excluded.output_tokens,cost_usd=excluded.cost_usd,
               failure_class=excluded.failure_class",
            params![
                run_id,
                trial.campaign_id,
                trial.id,
                trial.suite_id,
                trial.case_id,
                trial.arm,
                outcome_str(trial.outcome),
                i64::from(trial.attempt),
                i64::try_from(trial.duration_ms)?,
                i64::from(trial.model_calls),
                i64::try_from(trial.tool_calls)?,
                trial
                    .input_tokens
                    .and_then(|value| i64::try_from(value).ok()),
                trial
                    .output_tokens
                    .and_then(|value| i64::try_from(value).ok()),
                trial.cost_usd,
                trial.failure_class,
            ],
        )?;
        transaction.execute(
            "UPDATE eval_campaigns SET status=CASE WHEN status='completed' THEN status ELSE 'running' END,
               passed_trials=(SELECT COUNT(*) FROM eval_trials WHERE experiment_id=?1 AND campaign_id=?2 AND outcome='passed'),
               infra_error_trials=(SELECT COUNT(*) FROM eval_trials WHERE experiment_id=?1 AND campaign_id=?2 AND outcome IN ('infra_error','simulator_error')),
               failed_trials=(SELECT COUNT(*) FROM eval_trials WHERE experiment_id=?1 AND campaign_id=?2 AND outcome NOT IN ('passed','infra_error','simulator_error')),
               duration_ms=(SELECT COALESCE(SUM(duration_ms),0) FROM eval_trials WHERE experiment_id=?1 AND campaign_id=?2),
               cost_usd=(SELECT CASE WHEN COUNT(*)=COUNT(cost_usd) THEN SUM(cost_usd) END FROM eval_trials WHERE experiment_id=?1 AND campaign_id=?2)
             WHERE experiment_id=?1 AND id=?2",
            params![run_id, trial.campaign_id],
        )?;
        transaction.execute(
            "UPDATE eval_experiments SET
               completed_trials=(SELECT COUNT(*) FROM eval_trials WHERE experiment_id=?1),
               passed_trials=(SELECT COUNT(*) FROM eval_trials WHERE experiment_id=?1 AND outcome='passed'),
               infra_error_trials=(SELECT COUNT(*) FROM eval_trials WHERE experiment_id=?1 AND outcome IN ('infra_error','simulator_error')),
               failed_trials=(SELECT COUNT(*) FROM eval_trials WHERE experiment_id=?1 AND outcome NOT IN ('passed','infra_error','simulator_error')),
               observed_cost_usd=(SELECT CASE WHEN COUNT(*)=COUNT(cost_usd) THEN SUM(cost_usd) END FROM eval_trials WHERE experiment_id=?1)
             WHERE id=?1",
            [run_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn index_evidence(
        &self,
        run_id: &str,
        evidence: &ModelCampaignEvidence,
        artifact: &StoredEvalArtifact,
    ) -> Result<()> {
        let (total, passed, failed, infra) = evidence_summary(evidence);
        let cost = evidence
            .trial_results
            .iter()
            .map(|trial| trial.cost.total_usd)
            .try_fold(0.0, |sum, value| value.map(|value| sum + value));
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT OR IGNORE INTO eval_artifacts(
               sha256,size_bytes,media_type,created_at,pinned,protected
             ) VALUES(?1,?2,'application/json',?3,0,?4)",
            params![
                artifact.sha256,
                i64::try_from(artifact.size_bytes)?,
                now(),
                evidence.source.is_release_eligible(),
            ],
        )?;
        transaction.execute(
            "INSERT OR REPLACE INTO eval_artifact_refs(
               owner_kind,owner_id,artifact_kind,sha256,retention_until
             ) VALUES('campaign',?1,'evidence',?2,?3)",
            params![
                format!("{run_id}:{}", evidence.campaign_id),
                artifact.sha256,
                if evidence.source.is_release_eligible() {
                    None
                } else {
                    Some(retention_at(90))
                },
            ],
        )?;
        transaction.execute(
            "UPDATE eval_campaigns SET status='completed',evidence_artifact_sha256=?3,
               aggregate_status=?4,total_trials=?5,passed_trials=?6,failed_trials=?7,
               infra_error_trials=?8,duration_ms=?9,cost_usd=?10
             WHERE experiment_id=?1 AND id=?2",
            params![
                run_id,
                evidence.campaign_id,
                artifact.sha256,
                format!("{:?}", evidence.aggregate_status).to_ascii_lowercase(),
                i64::from(total),
                i64::from(passed),
                i64::from(failed),
                i64::from(infra),
                i64::try_from(evidence.duration_ms)?,
                cost,
            ],
        )?;
        transaction.execute(
            "DELETE FROM eval_trials WHERE experiment_id=?1 AND campaign_id=?2",
            params![run_id, evidence.campaign_id],
        )?;
        for trial in &evidence.trial_results {
            transaction.execute(
                "INSERT INTO eval_trials(
                   experiment_id,campaign_id,id,suite_id,case_id,arm,outcome,attempt,
                   duration_ms,model_calls,tool_calls,input_tokens,output_tokens,cost_usd,
                   trace_artifact_sha256,failure_class
                 ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,NULL,?15)",
                params![
                    run_id,
                    evidence.campaign_id,
                    trial.trial_id,
                    trial.suite_id,
                    trial.case_id,
                    trial.arm,
                    outcome_str(trial.outcome),
                    i64::from(trial.attempt),
                    i64::try_from(trial.timings.wall_ms)?,
                    i64::try_from(trial.orchestration.model_calls)?,
                    i64::try_from(trial.tools.attempted)?,
                    trial
                        .tokens
                        .input
                        .and_then(|value| i64::try_from(value).ok()),
                    trial
                        .tokens
                        .output
                        .and_then(|value| i64::try_from(value).ok()),
                    trial.cost.total_usd,
                    trial.failure_class,
                ],
            )?;
        }
        transaction.execute(
            "UPDATE eval_experiments SET
               completed_trials=(SELECT COALESCE(SUM(total_trials),0) FROM eval_campaigns WHERE experiment_id=?1 AND status='completed'),
               passed_trials=(SELECT COALESCE(SUM(passed_trials),0) FROM eval_campaigns WHERE experiment_id=?1),
               failed_trials=(SELECT COALESCE(SUM(failed_trials),0) FROM eval_campaigns WHERE experiment_id=?1),
               infra_error_trials=(SELECT COALESCE(SUM(infra_error_trials),0) FROM eval_campaigns WHERE experiment_id=?1),
               observed_cost_usd=(SELECT CASE WHEN COUNT(cost_usd)=COUNT(*) THEN SUM(cost_usd) END FROM eval_campaigns WHERE experiment_id=?1)
             WHERE id=?1",
            [run_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn experiment_debug_retention(&self, run_id: &str) -> Result<AppDebugRetention> {
        let request: Option<String> = self
            .connection()?
            .query_row(
                "SELECT request_json FROM eval_experiments WHERE id=?1",
                [run_id],
                |row| row.get(0),
            )
            .optional()?;
        let request = request.ok_or_else(|| anyhow!("evaluation experiment not found"))?;
        let request: EvalAppRunRequest = serde_json::from_str(&request)?;
        Ok(request.debug_retention)
    }

    pub fn experiment_request(&self, run_id: &str) -> Result<EvalAppRunRequest> {
        let request: Option<String> = self
            .connection()?
            .query_row(
                "SELECT request_json FROM eval_experiments WHERE id=?1",
                [run_id],
                |row| row.get(0),
            )
            .optional()?;
        serde_json::from_str(
            &request.ok_or_else(|| anyhow!("evaluation experiment has no retryable request"))?,
        )
        .map_err(Into::into)
    }

    pub fn index_run_artifact(
        &self,
        run_id: &str,
        campaign_id: &str,
        kind: &str,
        artifact: &StoredEvalArtifact,
        retention_days: u16,
    ) -> Result<()> {
        if kind.is_empty()
            || kind.len() > 64
            || !kind
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            bail!("invalid evaluation artifact kind");
        }
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT OR IGNORE INTO eval_artifacts(
               sha256,size_bytes,media_type,created_at,pinned,protected
             ) VALUES(?1,?2,'application/octet-stream',?3,0,0)",
            params![artifact.sha256, i64::try_from(artifact.size_bytes)?, now()],
        )?;
        transaction.execute(
            "INSERT OR REPLACE INTO eval_artifact_refs(
               owner_kind,owner_id,artifact_kind,sha256,retention_until
             ) VALUES('campaign',?1,?2,?3,?4)",
            params![
                format!("{run_id}:{campaign_id}"),
                kind,
                artifact.sha256,
                retention_at(retention_days),
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn expired_artifact_sha256s(&self) -> Result<Vec<String>> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM eval_artifact_refs
             WHERE retention_until IS NOT NULL AND retention_until <= ?1",
            [now()],
        )?;
        let values = {
            let mut statement = transaction.prepare(
                "SELECT a.sha256 FROM eval_artifacts a
                 WHERE a.pinned=0 AND a.protected=0
                   AND NOT EXISTS(
                     SELECT 1 FROM eval_artifact_refs r WHERE r.sha256=a.sha256
                   ) ORDER BY a.sha256",
            )?;
            let values = statement
                .query_map([], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            values
        };
        transaction.commit()?;
        Ok(values)
    }

    pub fn forget_collectable_artifact(&self, sha256: &str) -> Result<bool> {
        let changed = self.connection()?.execute(
            "DELETE FROM eval_artifacts
             WHERE sha256=?1 AND pinned=0 AND protected=0
               AND NOT EXISTS(
                 SELECT 1 FROM eval_artifact_refs r WHERE r.sha256=eval_artifacts.sha256
               )",
            [sha256],
        )?;
        Ok(changed == 1)
    }

    pub fn set_experiment_pinned(&self, experiment_id: &str, pinned: bool) -> Result<()> {
        let experiment = self
            .get_experiment(experiment_id)?
            .ok_or_else(|| anyhow!("evaluation experiment not found"))?;
        if matches!(
            experiment.integrity,
            EvalIntegrity::ProtectedVerified | EvalIntegrity::ProtectedUnknownAssets
        ) && !pinned
        {
            bail!("protected evidence remains pinned until an audited manual deletion");
        }
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "UPDATE eval_experiments SET pinned=?2 WHERE id=?1",
            params![experiment_id, pinned],
        )?;
        if pinned {
            transaction.execute(
                "UPDATE eval_artifacts SET pinned=1 WHERE sha256 IN(
                   SELECT sha256 FROM eval_artifact_refs
                   WHERE owner_kind='campaign' AND owner_id LIKE ?1
                 )",
                [format!("{experiment_id}:%")],
            )?;
        } else {
            transaction.execute(
                "UPDATE eval_artifacts AS artifact SET pinned=0
                 WHERE protected=0 AND sha256 IN(
                   SELECT sha256 FROM eval_artifact_refs
                   WHERE owner_kind='campaign' AND owner_id LIKE ?1
                 ) AND NOT EXISTS(
                   SELECT 1 FROM eval_artifact_refs ref
                   JOIN eval_experiments experiment
                     ON ref.owner_kind='campaign'
                    AND ref.owner_id LIKE experiment.id || ':%'
                   WHERE ref.sha256=artifact.sha256 AND experiment.pinned=1
                 )",
                [format!("{experiment_id}:%")],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn reconcile_interrupted(&self) -> Result<usize> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let changed = transaction.execute(
            "UPDATE eval_experiments SET status='interrupted',completed_at=?1,
               error='Evaluation process ended before a terminal event'
             WHERE status IN ('queued','planning','running','cancelling')",
            [now()],
        )?;
        transaction.execute(
            "UPDATE eval_campaigns AS campaign
             SET status=(
               SELECT experiment.status FROM eval_experiments AS experiment
               WHERE experiment.id=campaign.experiment_id
             )
             WHERE campaign.status!='completed' AND EXISTS (
               SELECT 1 FROM eval_experiments AS experiment
               WHERE experiment.id=campaign.experiment_id
                 AND experiment.status IN ('failed','cancelled','interrupted')
             )",
            [],
        )?;
        transaction.commit()?;
        Ok(changed)
    }

    pub fn get_experiment(&self, id: &str) -> Result<Option<EvalExperimentRecord>> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT id,kind,profile_id,source,integrity,status,git_ref,dirty,
                        app_version,plan_digest,parent_experiment_id,created_at,started_at,
                        completed_at,total_trials,completed_trials,passed_trials,failed_trials,
                        infra_error_trials,max_cost_usd,observed_cost_usd,pinned,
                        (SELECT signature_status FROM eval_imports
                         WHERE experiment_id=eval_experiments.id
                         ORDER BY CASE signature_status
                           WHEN 'verified' THEN 0
                           WHEN 'verified_retired' THEN 1
                           WHEN 'verified_now_revoked' THEN 2
                           WHEN 'verified_key_missing' THEN 3
                           ELSE 4 END, imported_at DESC LIMIT 1),error
                 FROM eval_experiments WHERE id=?1",
                [id],
                map_experiment,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_experiments(&self, query: &EvalHistoryQuery) -> Result<Vec<EvalExperimentRecord>> {
        let limit = query.limit.clamp(1, 200);
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id,kind,profile_id,source,integrity,status,git_ref,dirty,
                    app_version,plan_digest,parent_experiment_id,created_at,started_at,
                    completed_at,total_trials,completed_trials,passed_trials,failed_trials,
                    infra_error_trials,max_cost_usd,observed_cost_usd,pinned,
                    (SELECT signature_status FROM eval_imports
                     WHERE experiment_id=eval_experiments.id
                     ORDER BY CASE signature_status
                       WHEN 'verified' THEN 0
                       WHEN 'verified_retired' THEN 1
                       WHEN 'verified_now_revoked' THEN 2
                       WHEN 'verified_key_missing' THEN 3
                       ELSE 4 END, imported_at DESC LIMIT 1),error
             FROM eval_experiments
             WHERE (?1 IS NULL OR kind=?1) AND (?2 IS NULL OR source=?2)
               AND (?3 IS NULL OR status=?3)
             ORDER BY created_at DESC LIMIT ?4 OFFSET ?5",
        )?;
        let rows = statement.query_map(
            params![
                query.kind.map(kind_str),
                query.source.map(source_str),
                query.status.map(EvalExperimentStatus::as_str),
                i64::from(limit),
                i64::from(query.offset),
            ],
            map_experiment,
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn detail(&self, id: &str) -> Result<Option<EvalExperimentDetail>> {
        let Some(experiment) = self.get_experiment(id)? else {
            return Ok(None);
        };
        let connection = self.connection()?;
        let mut campaigns_statement = connection.prepare(
            "SELECT id,experiment_id,model_digest,provider_config_digest,status,
                    evidence_artifact_sha256,aggregate_status,total_trials,passed_trials,
                    failed_trials,infra_error_trials,duration_ms,cost_usd
             FROM eval_campaigns WHERE experiment_id=?1 ORDER BY id",
        )?;
        let campaigns = campaigns_statement
            .query_map([id], map_campaign)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut trials_statement = connection.prepare(
            "SELECT id,campaign_id,suite_id,case_id,arm,outcome,attempt,duration_ms,
                    model_calls,tool_calls,input_tokens,output_tokens,cost_usd,
                    trace_artifact_sha256,failure_class
             FROM eval_trials WHERE experiment_id=?1 ORDER BY campaign_id,suite_id,case_id,id",
        )?;
        let trials = trials_statement
            .query_map([id], map_trial)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Some(EvalExperimentDetail {
            experiment,
            campaigns,
            trials,
        }))
    }

    pub fn evidence_artifact_sha256s(&self, experiment_id: &str) -> Result<Vec<String>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT evidence_artifact_sha256 FROM eval_campaigns
             WHERE experiment_id=?1 AND evidence_artifact_sha256 IS NOT NULL ORDER BY id",
        )?;
        let rows = statement.query_map([experiment_id], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn campaign_evidence_artifacts(
        &self,
        experiment_id: &str,
    ) -> Result<Vec<(String, String)>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id,evidence_artifact_sha256 FROM eval_campaigns
             WHERE experiment_id=?1 AND evidence_artifact_sha256 IS NOT NULL ORDER BY id",
        )?;
        let rows = statement.query_map([experiment_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn campaign_evidence_sha256(
        &self,
        experiment_id: &str,
        campaign_id: &str,
    ) -> Result<Option<String>> {
        self.connection()?
            .query_row(
                "SELECT evidence_artifact_sha256 FROM eval_campaigns
                 WHERE experiment_id=?1 AND id=?2",
                params![experiment_id, campaign_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn create_baseline(
        &self,
        experiment_id: &str,
        tier: ModelCampaignTier,
        approved_by: &str,
        note: Option<&str>,
    ) -> Result<EvalBaselineRecord> {
        let experiment = self
            .get_experiment(experiment_id)?
            .ok_or_else(|| anyhow!("evaluation experiment not found"))?;
        if experiment.integrity != EvalIntegrity::ProtectedVerified
            || experiment.status != EvalExperimentStatus::Completed
            || !matches!(
                experiment.signature_status.as_deref(),
                Some("verified" | "verified_retired")
            )
        {
            bail!("only completed protected evidence with a currently trusted signature can become a baseline");
        }
        if experiment.profile_id != format!("protected:{}", tier_str(tier)) {
            bail!("baseline tier must match the protected evidence tier");
        }
        let approved_by = approved_by.trim();
        if approved_by.is_empty() || approved_by.len() > 128 {
            bail!("baseline approval actor must contain 1..128 characters");
        }
        let record = EvalBaselineRecord {
            id: format!("baseline-{}", uuid::Uuid::new_v4()),
            experiment_id: experiment_id.to_string(),
            tier,
            approved_by: approved_by.to_string(),
            approved_at: now(),
            note: note.map(str::to_string),
        };
        self.connection()?.execute(
            "INSERT INTO eval_baselines(id,experiment_id,tier,approved_by,approved_at,note)
             VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                record.id,
                record.experiment_id,
                tier_str(record.tier),
                record.approved_by,
                record.approved_at,
                record.note,
            ],
        )?;
        Ok(record)
    }

    pub fn delete_baseline(&self, id: &str) -> Result<bool> {
        Ok(self
            .connection()?
            .execute("DELETE FROM eval_baselines WHERE id=?1", [id])?
            > 0)
    }

    pub fn list_baselines(
        &self,
        tier: Option<ModelCampaignTier>,
    ) -> Result<Vec<EvalBaselineRecord>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id,experiment_id,tier,approved_by,approved_at,note FROM eval_baselines
             WHERE (?1 IS NULL OR tier=?1) ORDER BY approved_at DESC",
        )?;
        let rows = statement.query_map([tier.map(tier_str)], |row| {
            Ok(EvalBaselineRecord {
                id: row.get(0)?,
                experiment_id: row.get(1)?,
                tier: parse_tier(&row.get::<_, String>(2)?)?,
                approved_by: row.get(3)?,
                approved_at: row.get(4)?,
                note: row.get(5)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn create_annotation(
        &self,
        experiment_id: &str,
        campaign_id: Option<&str>,
        trial_id: Option<&str>,
        text: &str,
    ) -> Result<EvalAnnotationRecord> {
        if self.get_experiment(experiment_id)?.is_none() {
            bail!("evaluation experiment not found");
        }
        let text = text.trim();
        if text.is_empty() || text.len() > 4_000 {
            bail!("evaluation annotation must contain 1..=4000 bytes");
        }
        let connection = self.connection()?;
        if let Some(campaign_id) = campaign_id {
            let exists = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM eval_campaigns WHERE experiment_id=?1 AND id=?2)",
                params![experiment_id, campaign_id],
                |row| row.get::<_, bool>(0),
            )?;
            if !exists {
                bail!("evaluation annotation campaign does not belong to the experiment");
            }
        } else if trial_id.is_some() {
            bail!("trial annotations must identify their campaign");
        }
        if let (Some(campaign_id), Some(trial_id)) = (campaign_id, trial_id) {
            let exists = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM eval_trials
                  WHERE experiment_id=?1 AND campaign_id=?2 AND id=?3)",
                params![experiment_id, campaign_id, trial_id],
                |row| row.get::<_, bool>(0),
            )?;
            if !exists {
                bail!("evaluation annotation trial does not belong to the campaign");
            }
        }
        let record = EvalAnnotationRecord {
            id: format!("evalnote-{}", uuid::Uuid::new_v4()),
            experiment_id: experiment_id.to_string(),
            campaign_id: campaign_id.map(str::to_string),
            trial_id: trial_id.map(str::to_string),
            text: text.to_string(),
            created_at: now(),
        };
        connection.execute(
            "INSERT INTO eval_annotations(
               id,experiment_id,campaign_id,trial_id,text,created_at
             ) VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                record.id,
                record.experiment_id,
                record.campaign_id,
                record.trial_id,
                record.text,
                record.created_at,
            ],
        )?;
        Ok(record)
    }

    pub fn list_annotations(&self, experiment_id: &str) -> Result<Vec<EvalAnnotationRecord>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id,experiment_id,campaign_id,trial_id,text,created_at
             FROM eval_annotations WHERE experiment_id=?1 ORDER BY created_at,id",
        )?;
        let rows = statement.query_map([experiment_id], |row| {
            Ok(EvalAnnotationRecord {
                id: row.get(0)?,
                experiment_id: row.get(1)?,
                campaign_id: row.get(2)?,
                trial_id: row.get(3)?,
                text: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn import_unverified_evidence(
        &self,
        evidence_artifact: &StoredEvalArtifact,
        evidence: &ModelCampaignEvidence,
    ) -> Result<EvalImportResult> {
        let mut connection = self.connection()?;
        if let Some(existing) = connection
            .query_row(
                "SELECT id,experiment_id,evidence_sha256,key_id,integrity
                 FROM eval_imports WHERE bundle_sha256=?1",
                [&evidence_artifact.sha256],
                |row| {
                    Ok(EvalImportResult {
                        import_id: row.get(0)?,
                        experiment_id: row.get(1)?,
                        evidence_sha256: row.get(2)?,
                        key_id: row.get(3)?,
                        integrity: parse_integrity(&row.get::<_, String>(4)?)?,
                        already_imported: true,
                    })
                },
            )
            .optional()?
        {
            return Ok(existing);
        }

        let (total, passed, failed, infra) = evidence_summary(evidence);
        let experiment_id = format!("unverified-{}", &evidence_artifact.sha256[..24]);
        let import_id = format!("evalimport-{}", uuid::Uuid::new_v4());
        let observed_cost = evidence
            .trial_results
            .iter()
            .map(|trial| trial.cost.total_usd)
            .try_fold(0.0, |sum, value| value.map(|value| sum + value));
        let model_digest = evidence
            .trial_results
            .first()
            .map(|trial| trial.model_digest.clone())
            .unwrap_or_else(|| "0".repeat(64));
        let provider_config_digest = evidence
            .trial_results
            .first()
            .and_then(|trial| trial.runtime_config_digest.clone())
            .unwrap_or_else(|| "0".repeat(64));
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT OR IGNORE INTO eval_artifacts(
               sha256,size_bytes,media_type,created_at,pinned,protected
             ) VALUES(?1,?2,'application/json',?3,0,0)",
            params![
                evidence_artifact.sha256,
                i64::try_from(evidence_artifact.size_bytes)?,
                now(),
            ],
        )?;
        transaction.execute(
            "INSERT INTO eval_artifact_refs(
               owner_kind,owner_id,artifact_kind,sha256,retention_until
             ) VALUES('import',?1,'unverified_evidence',?2,?3)",
            params![import_id, evidence_artifact.sha256, retention_at(90)],
        )?;
        transaction.execute(
            "INSERT INTO eval_experiments(
               id,kind,profile_id,source,integrity,status,git_ref,dirty,app_version,
               plan_digest,runtime_json,created_at,started_at,completed_at,total_trials,
               completed_trials,passed_trials,failed_trials,infra_error_trials,
               observed_cost_usd,pinned
             ) VALUES(?1,'hope_core',?2,?3,'unverified_import','completed',?4,?5,?6,
               NULL,?7,?8,?8,?9,?10,?10,?11,?12,?13,?14,0)",
            params![
                experiment_id,
                format!("unverified:{:?}", evidence.tier).to_ascii_lowercase(),
                source_str(evidence.source),
                evidence.commit_sha,
                evidence.dirty,
                evidence.app_version,
                serde_json::to_string(&serde_json::json!({
                    "runnerOs": evidence.runner_os,
                    "runnerArch": evidence.runner_arch,
                    "runnerDigest": evidence.runner_digest,
                }))?,
                evidence.started_at,
                evidence.completed_at,
                i64::from(total),
                i64::from(passed),
                i64::from(failed),
                i64::from(infra),
                observed_cost,
            ],
        )?;
        transaction.execute(
            "INSERT INTO eval_campaigns(
               experiment_id,id,model_digest,provider_config_digest,status,
               evidence_artifact_sha256,aggregate_status,total_trials,passed_trials,
               failed_trials,infra_error_trials,duration_ms,cost_usd
             ) VALUES(?1,?2,?3,?4,'completed',?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                experiment_id,
                evidence.campaign_id,
                model_digest,
                provider_config_digest,
                evidence_artifact.sha256,
                format!("{:?}", evidence.aggregate_status).to_ascii_lowercase(),
                i64::from(total),
                i64::from(passed),
                i64::from(failed),
                i64::from(infra),
                i64::try_from(evidence.duration_ms)?,
                observed_cost,
            ],
        )?;
        for trial in &evidence.trial_results {
            transaction.execute(
                "INSERT INTO eval_trials(
                   experiment_id,campaign_id,id,suite_id,case_id,arm,outcome,attempt,
                   duration_ms,model_calls,tool_calls,input_tokens,output_tokens,cost_usd,
                   trace_artifact_sha256,failure_class
                 ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,NULL,?15)",
                params![
                    experiment_id,
                    evidence.campaign_id,
                    trial.trial_id,
                    trial.suite_id,
                    trial.case_id,
                    trial.arm,
                    outcome_str(trial.outcome),
                    i64::from(trial.attempt),
                    i64::try_from(trial.timings.wall_ms)?,
                    i64::try_from(trial.orchestration.model_calls)?,
                    i64::try_from(trial.tools.attempted)?,
                    trial
                        .tokens
                        .input
                        .and_then(|value| i64::try_from(value).ok()),
                    trial
                        .tokens
                        .output
                        .and_then(|value| i64::try_from(value).ok()),
                    trial.cost.total_usd,
                    trial.failure_class,
                ],
            )?;
        }
        transaction.execute(
            "INSERT INTO eval_imports(
               id,bundle_sha256,experiment_id,evidence_sha256,integrity,key_id,
               key_fingerprint,signature_status,imported_at
             ) VALUES(?1,?2,?3,?2,'unverified_import',NULL,NULL,'unsigned',?4)",
            params![import_id, evidence_artifact.sha256, experiment_id, now()],
        )?;
        transaction.commit()?;
        Ok(EvalImportResult {
            import_id,
            experiment_id,
            integrity: EvalIntegrity::UnverifiedImport,
            key_id: None,
            evidence_sha256: evidence_artifact.sha256.clone(),
            already_imported: false,
        })
    }

    /// Atomically index a verified protected bundle. Signature verification is
    /// intentionally performed by the bundle importer before this method;
    /// this repository only accepts the resulting protected identity together
    /// with content-addressed artifacts.
    pub fn import_protected_evidence(
        &self,
        bundle: &StoredEvalArtifact,
        evidence_artifact: &StoredEvalArtifact,
        extra_artifacts: &[StoredEvalArtifact],
        key_id: &str,
        key_fingerprint: &str,
        evidence: &ModelCampaignEvidence,
        assets_known: bool,
    ) -> Result<EvalImportResult> {
        if evidence.dirty || !evidence.source.is_release_eligible() {
            bail!("only clean release-eligible evidence can be imported as protected");
        }
        let integrity = if assets_known {
            EvalIntegrity::ProtectedVerified
        } else {
            EvalIntegrity::ProtectedUnknownAssets
        };
        let integrity_name = integrity_str(integrity);
        let signature_status = "verified";
        let mut connection = self.connection()?;
        if let Some(mut existing) = connection
            .query_row(
                "SELECT id,experiment_id,evidence_sha256,key_id,integrity FROM eval_imports WHERE bundle_sha256=?1",
                [&bundle.sha256],
                |row| {
                    Ok(EvalImportResult {
                        import_id: row.get(0)?,
                        experiment_id: row.get(1)?,
                        integrity: parse_integrity(&row.get::<_, String>(4)?)?,
                        evidence_sha256: row.get(2)?,
                        key_id: row.get(3)?,
                        already_imported: true,
                    })
                },
            )
            .optional()?
        {
            if assets_known && existing.integrity == EvalIntegrity::ProtectedUnknownAssets {
                let transaction = connection.transaction()?;
                transaction.execute(
                    "UPDATE eval_experiments SET integrity='protected_verified' WHERE id=?1",
                    [&existing.experiment_id],
                )?;
                transaction.execute(
                    "UPDATE eval_imports SET integrity='protected_verified' WHERE experiment_id=?1",
                    [&existing.experiment_id],
                )?;
                transaction.commit()?;
                existing.integrity = EvalIntegrity::ProtectedVerified;
            }
            connection.execute(
                "UPDATE eval_imports
                 SET key_id=?2,key_fingerprint=?3,signature_status='verified'
                 WHERE id=?1",
                params![existing.import_id, key_id, key_fingerprint],
            )?;
            return Ok(existing);
        }

        let existing_experiment = connection
            .query_row(
                "SELECT i.experiment_id,e.integrity FROM eval_imports i
                 JOIN eval_experiments e ON e.id=i.experiment_id
                 WHERE i.evidence_sha256=?1
                   AND e.integrity IN ('protected_verified','protected_unknown_assets')
                 LIMIT 1",
                [&evidence_artifact.sha256],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        parse_integrity(&row.get::<_, String>(1)?)?,
                    ))
                },
            )
            .optional()?;
        let import_id = format!("evalimport-{}", uuid::Uuid::new_v4());
        if let Some((experiment_id, existing_integrity)) = existing_experiment {
            let effective_integrity = if assets_known {
                EvalIntegrity::ProtectedVerified
            } else {
                existing_integrity
            };
            let existing_integrity_name = integrity_str(effective_integrity);
            let transaction = connection.transaction()?;
            if effective_integrity != existing_integrity {
                transaction.execute(
                    "UPDATE eval_experiments SET integrity=?2 WHERE id=?1",
                    params![experiment_id, existing_integrity_name],
                )?;
                transaction.execute(
                    "UPDATE eval_imports SET integrity=?2 WHERE experiment_id=?1",
                    params![experiment_id, existing_integrity_name],
                )?;
            }
            transaction.execute(
                "INSERT OR IGNORE INTO eval_artifacts(
                   sha256,size_bytes,media_type,created_at,pinned,protected
                 ) VALUES(?1,?2,'application/zip',?3,1,1)",
                params![bundle.sha256, i64::try_from(bundle.size_bytes)?, now()],
            )?;
            transaction.execute(
                "INSERT INTO eval_imports(
                   id,bundle_sha256,experiment_id,evidence_sha256,integrity,key_id,
                   key_fingerprint,signature_status,imported_at
                 ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    import_id,
                    bundle.sha256,
                    experiment_id,
                    evidence_artifact.sha256,
                    existing_integrity_name,
                    key_id,
                    key_fingerprint,
                    signature_status,
                    now(),
                ],
            )?;
            transaction.execute(
                "INSERT OR IGNORE INTO eval_artifact_refs(
                   owner_kind,owner_id,artifact_kind,sha256,retention_until
                 ) VALUES('import',?1,'protected_bundle',?2,NULL)",
                params![import_id, bundle.sha256],
            )?;
            transaction.commit()?;
            return Ok(EvalImportResult {
                import_id,
                experiment_id,
                integrity: effective_integrity,
                key_id: Some(key_id.to_string()),
                evidence_sha256: evidence_artifact.sha256.clone(),
                already_imported: true,
            });
        }

        let (total, passed, failed, infra) = evidence_summary(evidence);
        let experiment_id = format!("protected-{}", &evidence_artifact.sha256[..24]);
        let observed_cost = evidence
            .trial_results
            .iter()
            .map(|trial| trial.cost.total_usd)
            .try_fold(0.0, |sum, value| value.map(|value| sum + value));
        let model_digest = evidence
            .trial_results
            .first()
            .map(|trial| trial.model_digest.clone())
            .unwrap_or_else(|| "0".repeat(64));
        let provider_config_digest = evidence
            .trial_results
            .first()
            .and_then(|trial| trial.runtime_config_digest.clone())
            .unwrap_or_else(|| "0".repeat(64));
        let transaction = connection.transaction()?;
        for (index, artifact) in std::iter::once(bundle)
            .chain(std::iter::once(evidence_artifact))
            .chain(extra_artifacts.iter())
            .enumerate()
        {
            transaction.execute(
                "INSERT OR IGNORE INTO eval_artifacts(
                   sha256,size_bytes,media_type,created_at,pinned,protected
                 ) VALUES(?1,?2,'application/octet-stream',?3,1,1)",
                params![artifact.sha256, i64::try_from(artifact.size_bytes)?, now(),],
            )?;
            transaction.execute(
                "INSERT OR IGNORE INTO eval_artifact_refs(
                   owner_kind,owner_id,artifact_kind,sha256,retention_until
                 ) VALUES('import',?1,?2,?3,NULL)",
                params![import_id, format!("protected_{index}"), artifact.sha256],
            )?;
        }
        transaction.execute(
            "INSERT INTO eval_experiments(
               id,kind,profile_id,source,integrity,status,git_ref,dirty,app_version,
               plan_digest,runtime_json,created_at,started_at,completed_at,total_trials,
               completed_trials,passed_trials,failed_trials,infra_error_trials,observed_cost_usd,pinned
             ) VALUES(?1,'hope_core',?2,?3,?4,'completed',?5,0,?6,
               NULL,?7,?8,?9,?10,?11,?11,?12,?13,?14,?15,1)",
            params![
                experiment_id,
                format!("protected:{:?}", evidence.tier).to_ascii_lowercase(),
                source_str(evidence.source),
                integrity_name,
                evidence.commit_sha,
                evidence.app_version,
                serde_json::to_string(&serde_json::json!({
                    "runnerOs": evidence.runner_os,
                    "runnerArch": evidence.runner_arch,
                    "runnerDigest": evidence.runner_digest,
                }))?,
                evidence.started_at,
                evidence.started_at,
                evidence.completed_at,
                i64::from(total),
                i64::from(passed),
                i64::from(failed),
                i64::from(infra),
                observed_cost,
            ],
        )?;
        transaction.execute(
            "INSERT INTO eval_campaigns(
               experiment_id,id,model_digest,provider_config_digest,status,
               evidence_artifact_sha256,aggregate_status,total_trials,passed_trials,
               failed_trials,infra_error_trials,duration_ms,cost_usd
             ) VALUES(?1,?2,?3,?4,'completed',?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                experiment_id,
                evidence.campaign_id,
                model_digest,
                provider_config_digest,
                evidence_artifact.sha256,
                format!("{:?}", evidence.aggregate_status).to_ascii_lowercase(),
                i64::from(total),
                i64::from(passed),
                i64::from(failed),
                i64::from(infra),
                i64::try_from(evidence.duration_ms)?,
                observed_cost,
            ],
        )?;
        for trial in &evidence.trial_results {
            transaction.execute(
                "INSERT INTO eval_trials(
                   experiment_id,campaign_id,id,suite_id,case_id,arm,outcome,attempt,
                   duration_ms,model_calls,tool_calls,input_tokens,output_tokens,cost_usd,
                   trace_artifact_sha256,failure_class
                 ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,NULL,?15)",
                params![
                    experiment_id,
                    evidence.campaign_id,
                    trial.trial_id,
                    trial.suite_id,
                    trial.case_id,
                    trial.arm,
                    outcome_str(trial.outcome),
                    i64::from(trial.attempt),
                    i64::try_from(trial.timings.wall_ms)?,
                    i64::try_from(trial.orchestration.model_calls)?,
                    i64::try_from(trial.tools.attempted)?,
                    trial
                        .tokens
                        .input
                        .and_then(|value| i64::try_from(value).ok()),
                    trial
                        .tokens
                        .output
                        .and_then(|value| i64::try_from(value).ok()),
                    trial.cost.total_usd,
                    trial.failure_class,
                ],
            )?;
        }
        transaction.execute(
            "INSERT INTO eval_imports(
               id,bundle_sha256,experiment_id,evidence_sha256,integrity,key_id,
               key_fingerprint,signature_status,imported_at
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                import_id,
                bundle.sha256,
                experiment_id,
                evidence_artifact.sha256,
                integrity_name,
                key_id,
                key_fingerprint,
                signature_status,
                now(),
            ],
        )?;
        transaction.commit()?;
        Ok(EvalImportResult {
            import_id,
            experiment_id,
            integrity,
            key_id: Some(key_id.to_string()),
            evidence_sha256: evidence_artifact.sha256.clone(),
            already_imported: false,
        })
    }

    /// Reconcile the mutable trust-registry view without changing immutable
    /// evidence provenance. A key that is retired remains valid for evidence
    /// already verified inside its signing window; a revoked or removed key
    /// immediately removes baseline eligibility while preserving history.
    pub fn refresh_import_signature_status(&self, trust: &EvidenceTrustRegistry) -> Result<usize> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id,key_id,key_fingerprint FROM eval_imports
             WHERE integrity IN ('protected_verified','protected_unknown_assets')",
        )?;
        let imports = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);

        let mut changed = 0usize;
        for (import_id, key_id, stored_fingerprint) in imports {
            let status = match key_id
                .as_deref()
                .and_then(|id| trust.keys.iter().find(|key| key.id == id))
            {
                None => "verified_key_missing",
                Some(_) if stored_fingerprint.is_none() => "verified_key_identity_missing",
                Some(key) => match evidence_trust_key_fingerprint(key) {
                    Ok(current) if stored_fingerprint.as_deref() == Some(current.as_str()) => {
                        match key.status {
                            EvidenceKeyStatus::Active => "verified",
                            EvidenceKeyStatus::Retired => "verified_retired",
                            EvidenceKeyStatus::Revoked => "verified_now_revoked",
                        }
                    }
                    _ => "verified_key_mismatch",
                },
            };
            changed += connection.execute(
                "UPDATE eval_imports SET signature_status=?2
                 WHERE id=?1 AND signature_status<>?2",
                params![import_id, status],
            )?;
        }
        Ok(changed)
    }

    pub fn mark_import_signature_keys_missing(&self) -> Result<usize> {
        Ok(self.connection()?.execute(
            "UPDATE eval_imports SET signature_status='verified_key_missing'
             WHERE integrity IN ('protected_verified','protected_unknown_assets')
               AND signature_status<>'verified_key_missing'",
            [],
        )?)
    }
}

fn map_experiment(row: &rusqlite::Row<'_>) -> rusqlite::Result<EvalExperimentRecord> {
    Ok(EvalExperimentRecord {
        id: row.get(0)?,
        kind: parse_kind(&row.get::<_, String>(1)?)?,
        profile_id: row.get(2)?,
        source: parse_source(&row.get::<_, String>(3)?)?,
        integrity: parse_integrity(&row.get::<_, String>(4)?)?,
        status: parse_status(&row.get::<_, String>(5)?)?,
        reference: row.get(6)?,
        dirty: row.get(7)?,
        app_version: row.get(8)?,
        plan_digest: row.get(9)?,
        parent_experiment_id: row.get(10)?,
        created_at: row.get(11)?,
        started_at: row.get(12)?,
        completed_at: row.get(13)?,
        total_trials: sql_u32(row, 14)?,
        completed_trials: sql_u32(row, 15)?,
        passed_trials: sql_u32(row, 16)?,
        failed_trials: sql_u32(row, 17)?,
        infra_error_trials: sql_u32(row, 18)?,
        max_cost_usd: row.get(19)?,
        observed_cost_usd: row.get(20)?,
        pinned: row.get(21)?,
        signature_status: row.get(22)?,
        error: row.get(23)?,
    })
}

fn map_campaign(row: &rusqlite::Row<'_>) -> rusqlite::Result<EvalCampaignRecord> {
    Ok(EvalCampaignRecord {
        id: row.get(0)?,
        experiment_id: row.get(1)?,
        model_digest: row.get(2)?,
        provider_config_digest: row.get(3)?,
        status: parse_status(&row.get::<_, String>(4)?)?,
        evidence_artifact_sha256: row.get(5)?,
        aggregate_status: row.get(6)?,
        total_trials: sql_u32(row, 7)?,
        passed_trials: sql_u32(row, 8)?,
        failed_trials: sql_u32(row, 9)?,
        infra_error_trials: sql_u32(row, 10)?,
        duration_ms: row
            .get::<_, Option<i64>>(11)?
            .map(u64::try_from)
            .transpose()
            .map_err(sql_conversion)?,
        cost_usd: row.get(12)?,
    })
}

fn map_trial(row: &rusqlite::Row<'_>) -> rusqlite::Result<EvalTrialRecord> {
    Ok(EvalTrialRecord {
        id: row.get(0)?,
        campaign_id: row.get(1)?,
        suite_id: row.get(2)?,
        case_id: row.get(3)?,
        arm: row.get(4)?,
        outcome: parse_outcome(&row.get::<_, String>(5)?)?,
        attempt: u8::try_from(row.get::<_, i64>(6)?).map_err(sql_conversion)?,
        duration_ms: u64::try_from(row.get::<_, i64>(7)?).map_err(sql_conversion)?,
        model_calls: sql_u32(row, 8)?,
        tool_calls: u64::try_from(row.get::<_, i64>(9)?).map_err(sql_conversion)?,
        input_tokens: row
            .get::<_, Option<i64>>(10)?
            .map(u64::try_from)
            .transpose()
            .map_err(sql_conversion)?,
        output_tokens: row
            .get::<_, Option<i64>>(11)?
            .map(u64::try_from)
            .transpose()
            .map_err(sql_conversion)?,
        cost_usd: row.get(12)?,
        trace_artifact_sha256: row.get(13)?,
        failure_class: row.get(14)?,
    })
}

fn valid_transition(from: EvalExperimentStatus, to: EvalExperimentStatus) -> bool {
    use EvalExperimentStatus as S;
    matches!(
        (from, to),
        (
            S::Queued,
            S::Planning | S::Cancelled | S::Failed | S::Interrupted
        ) | (
            S::Planning,
            S::Running | S::Cancelling | S::Failed | S::Interrupted
        ) | (
            S::Running,
            S::Cancelling | S::Completed | S::Failed | S::Interrupted
        ) | (S::Cancelling, S::Cancelled | S::Failed | S::Interrupted)
    )
}

fn validate_run_id(id: &str) -> Result<()> {
    if id.len() > 96
        || !id.starts_with("evalrun-")
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        bail!("invalid evaluation run id");
    }
    Ok(())
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn retention_at(days: u16) -> String {
    (Utc::now() + Duration::days(i64::from(days))).to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn kind_str(value: EvalHistoryKind) -> &'static str {
    match value {
        EvalHistoryKind::HopeCore => "hope_core",
        EvalHistoryKind::Coding => "coding",
        EvalHistoryKind::Domain => "domain",
    }
}

fn source_str(value: ModelCampaignSource) -> &'static str {
    match value {
        ModelCampaignSource::GithubActions => "github_actions",
        ModelCampaignSource::DedicatedRunner => "dedicated_runner",
        ModelCampaignSource::LocalCli => "local_cli",
        ModelCampaignSource::LocalApp => "local_app",
    }
}

fn integrity_str(value: EvalIntegrity) -> &'static str {
    match value {
        EvalIntegrity::LocalDiagnostic => "local_diagnostic",
        EvalIntegrity::ProtectedVerified => "protected_verified",
        EvalIntegrity::ProtectedUnknownAssets => "protected_unknown_assets",
        EvalIntegrity::UnverifiedImport => "unverified_import",
        EvalIntegrity::LegacyLocal => "legacy_local",
    }
}

fn tier_str(value: ModelCampaignTier) -> &'static str {
    match value {
        ModelCampaignTier::Nightly => "nightly",
        ModelCampaignTier::Weekly => "weekly",
        ModelCampaignTier::Release => "release",
        ModelCampaignTier::Monthly => "monthly",
    }
}

fn outcome_str(value: ModelCampaignOutcome) -> &'static str {
    match value {
        ModelCampaignOutcome::Passed => "passed",
        ModelCampaignOutcome::TaskFailed => "task_failed",
        ModelCampaignOutcome::PolicyFailed => "policy_failed",
        ModelCampaignOutcome::BudgetExhausted => "budget_exhausted",
        ModelCampaignOutcome::InfraError => "infra_error",
        ModelCampaignOutcome::BenchmarkDefect => "benchmark_defect",
        ModelCampaignOutcome::SimulatorError => "simulator_error",
        ModelCampaignOutcome::Cancelled => "cancelled",
    }
}

fn parse_kind(value: &str) -> rusqlite::Result<EvalHistoryKind> {
    parse_enum(
        value,
        &[
            ("hope_core", EvalHistoryKind::HopeCore),
            ("coding", EvalHistoryKind::Coding),
            ("domain", EvalHistoryKind::Domain),
        ],
    )
}

fn parse_source(value: &str) -> rusqlite::Result<ModelCampaignSource> {
    parse_enum(
        value,
        &[
            ("github_actions", ModelCampaignSource::GithubActions),
            ("dedicated_runner", ModelCampaignSource::DedicatedRunner),
            ("local_cli", ModelCampaignSource::LocalCli),
            ("local_app", ModelCampaignSource::LocalApp),
        ],
    )
}

fn parse_integrity(value: &str) -> rusqlite::Result<EvalIntegrity> {
    parse_enum(
        value,
        &[
            ("local_diagnostic", EvalIntegrity::LocalDiagnostic),
            ("protected_verified", EvalIntegrity::ProtectedVerified),
            (
                "protected_unknown_assets",
                EvalIntegrity::ProtectedUnknownAssets,
            ),
            ("unverified_import", EvalIntegrity::UnverifiedImport),
            ("legacy_local", EvalIntegrity::LegacyLocal),
        ],
    )
}

fn parse_status(value: &str) -> rusqlite::Result<EvalExperimentStatus> {
    parse_enum(
        value,
        &[
            ("queued", EvalExperimentStatus::Queued),
            ("planning", EvalExperimentStatus::Planning),
            ("running", EvalExperimentStatus::Running),
            ("cancelling", EvalExperimentStatus::Cancelling),
            ("completed", EvalExperimentStatus::Completed),
            ("failed", EvalExperimentStatus::Failed),
            ("cancelled", EvalExperimentStatus::Cancelled),
            ("interrupted", EvalExperimentStatus::Interrupted),
        ],
    )
}

fn parse_outcome(value: &str) -> rusqlite::Result<ModelCampaignOutcome> {
    parse_enum(
        value,
        &[
            ("passed", ModelCampaignOutcome::Passed),
            ("task_failed", ModelCampaignOutcome::TaskFailed),
            ("policy_failed", ModelCampaignOutcome::PolicyFailed),
            ("budget_exhausted", ModelCampaignOutcome::BudgetExhausted),
            ("infra_error", ModelCampaignOutcome::InfraError),
            ("benchmark_defect", ModelCampaignOutcome::BenchmarkDefect),
            ("simulator_error", ModelCampaignOutcome::SimulatorError),
            ("cancelled", ModelCampaignOutcome::Cancelled),
        ],
    )
}

fn parse_tier(value: &str) -> rusqlite::Result<ModelCampaignTier> {
    parse_enum(
        value,
        &[
            ("nightly", ModelCampaignTier::Nightly),
            ("weekly", ModelCampaignTier::Weekly),
            ("release", ModelCampaignTier::Release),
            ("monthly", ModelCampaignTier::Monthly),
        ],
    )
}

fn parse_enum<T: Copy>(value: &str, values: &[(&str, T)]) -> rusqlite::Result<T> {
    values
        .iter()
        .find_map(|(name, parsed)| (*name == value).then_some(*parsed))
        .ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                format!("unknown evaluation enum value {value}").into(),
            )
        })
}

fn sql_u32(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u32> {
    u32::try_from(row.get::<_, i64>(index)?).map_err(sql_conversion)
}

fn sql_conversion(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Integer, Box::new(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use ha_eval_spec::app::{EvidenceTrustKey, EVIDENCE_TRUST_SCHEMA_VERSION};

    #[test]
    fn migration_is_idempotent_and_reconciliation_is_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("evals.db");
        let first = EvalRepository::open(path.clone()).unwrap();
        EvalRepository::open(path).unwrap();
        let connection = first.connection().unwrap();
        connection.execute(
            "INSERT INTO eval_experiments(id,kind,profile_id,source,integrity,status,git_ref,dirty,app_version,created_at)
             VALUES('evalrun-test','hope_core','quick','local_app','local_diagnostic','running','abc',1,'0.1.0',?1)",
            [now()],
        ).unwrap();
        assert_eq!(first.reconcile_interrupted().unwrap(), 1);
        assert_eq!(
            first
                .get_experiment("evalrun-test")
                .unwrap()
                .unwrap()
                .status,
            EvalExperimentStatus::Interrupted
        );
    }

    #[test]
    fn partial_trial_progress_survives_terminal_failure() {
        let temp = tempfile::tempdir().unwrap();
        let repository = EvalRepository::open(temp.path().join("evals.db")).unwrap();
        let connection = repository.connection().unwrap();
        connection
            .execute(
                "INSERT INTO eval_experiments(
                   id,kind,profile_id,source,integrity,status,git_ref,dirty,app_version,
                   created_at,started_at,total_trials
                 ) VALUES(
                   'evalrun-partial','hope_core','quick','local_app','local_diagnostic',
                   'running','abc',1,'0.1.0',?1,?1,2
                 )",
                [now()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO eval_campaigns(
                   experiment_id,id,model_digest,provider_config_digest,status,total_trials
                 ) VALUES('evalrun-partial','campaign','model','provider','running',2)",
                [],
            )
            .unwrap();
        drop(connection);

        repository
            .record_trial_progress(
                "evalrun-partial",
                &EvalTrialRecord {
                    id: "trial-1".to_string(),
                    campaign_id: "campaign".to_string(),
                    suite_id: "suite".to_string(),
                    case_id: "case".to_string(),
                    arm: "control".to_string(),
                    outcome: ModelCampaignOutcome::BudgetExhausted,
                    attempt: 1,
                    duration_ms: 180_000,
                    model_calls: 1,
                    tool_calls: 2,
                    input_tokens: Some(100),
                    output_tokens: Some(20),
                    cost_usd: Some(0.25),
                    trace_artifact_sha256: None,
                    failure_class: Some("trial_wall_timeout".to_string()),
                },
            )
            .unwrap();
        repository
            .transition(
                "evalrun-partial",
                EvalExperimentStatus::Failed,
                Some("experiment wall-clock budget exhausted"),
            )
            .unwrap();

        let detail = repository.detail("evalrun-partial").unwrap().unwrap();
        assert_eq!(detail.experiment.status, EvalExperimentStatus::Failed);
        assert_eq!(detail.experiment.completed_trials, 1);
        assert_eq!(detail.experiment.failed_trials, 1);
        assert_eq!(detail.campaigns[0].status, EvalExperimentStatus::Failed);
        assert_eq!(detail.trials.len(), 1);
        assert_eq!(
            detail.trials[0].outcome,
            ModelCampaignOutcome::BudgetExhausted
        );
        assert_eq!(detail.trials[0].model_calls, 1);
        assert_eq!(detail.trials[0].tool_calls, 2);
    }

    #[test]
    fn trust_reconciliation_preserves_history_but_revokes_baseline_eligibility() {
        let temp = tempfile::tempdir().unwrap();
        let repository = EvalRepository::open(temp.path().join("evals.db")).unwrap();
        let public_key = [7u8; 32];
        let key_fingerprint = ha_eval_spec::sha256_bytes(&public_key);
        let connection = repository.connection().unwrap();
        connection
            .execute(
                "INSERT INTO eval_experiments(
               id,kind,profile_id,source,integrity,status,git_ref,dirty,app_version,
               created_at,completed_at,total_trials,completed_trials,passed_trials
             ) VALUES(
               'protected-test','hope_core','protected:weekly','github_actions',
               'protected_verified','completed',?1,0,'0.1.0',?2,?2,1,1,1
             )",
                params!["a".repeat(40), now()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO eval_imports(
               id,bundle_sha256,experiment_id,evidence_sha256,integrity,key_id,
               key_fingerprint,signature_status,imported_at
             ) VALUES('import-test',?1,'protected-test',?2,'protected_verified',
               'release-key',?3,'verified',?4)",
                params!["b".repeat(64), "c".repeat(64), key_fingerprint, now()],
            )
            .unwrap();
        drop(connection);

        let mut registry = EvidenceTrustRegistry {
            schema_version: EVIDENCE_TRUST_SCHEMA_VERSION.to_string(),
            version: "1.0.0".to_string(),
            keys: vec![EvidenceTrustKey {
                id: "release-key".to_string(),
                algorithm: "ed25519".to_string(),
                public_key: base64::engine::general_purpose::STANDARD.encode(public_key),
                status: EvidenceKeyStatus::Active,
                valid_from: now(),
                valid_until: None,
                revoked_at: None,
            }],
        };
        repository
            .refresh_import_signature_status(&registry)
            .unwrap();
        assert_eq!(
            repository
                .get_experiment("protected-test")
                .unwrap()
                .unwrap()
                .signature_status
                .as_deref(),
            Some("verified")
        );
        repository
            .create_baseline(
                "protected-test",
                ModelCampaignTier::Weekly,
                "test_owner",
                None,
            )
            .unwrap();

        registry.keys[0].public_key = base64::engine::general_purpose::STANDARD.encode([8u8; 32]);
        repository
            .refresh_import_signature_status(&registry)
            .unwrap();
        assert_eq!(
            repository
                .get_experiment("protected-test")
                .unwrap()
                .unwrap()
                .signature_status
                .as_deref(),
            Some("verified_key_mismatch")
        );
        assert!(repository
            .create_baseline(
                "protected-test",
                ModelCampaignTier::Weekly,
                "test_owner",
                None,
            )
            .is_err());

        registry.keys[0].public_key = base64::engine::general_purpose::STANDARD.encode(public_key);

        registry.keys[0].status = EvidenceKeyStatus::Revoked;
        registry.keys[0].revoked_at = Some(now());
        repository
            .refresh_import_signature_status(&registry)
            .unwrap();
        let experiment = repository
            .get_experiment("protected-test")
            .unwrap()
            .unwrap();
        assert_eq!(experiment.integrity, EvalIntegrity::ProtectedVerified);
        assert_eq!(
            experiment.signature_status.as_deref(),
            Some("verified_now_revoked")
        );
        assert!(repository
            .create_baseline(
                "protected-test",
                ModelCampaignTier::Weekly,
                "test_owner",
                None,
            )
            .is_err());
    }
}
