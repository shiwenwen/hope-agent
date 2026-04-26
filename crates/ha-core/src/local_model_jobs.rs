//! Background jobs for Ollama install/start/model pulls.
//!
//! This is intentionally separate from `async_jobs`: those rows are tool-call
//! results that get injected back into chat sessions, while local model jobs
//! are user-visible setup tasks with a global task center.

use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::Instant;
use tokio_util::sync::CancellationToken;

const PROGRESS_THROTTLE_MS: u128 = 250;

use crate::agent::AssistantAgent;
use crate::local_embedding::{self, OllamaEmbeddingModel};
use crate::local_llm::{
    self, install_ollama_via_script_cancellable, start_ollama, InstallScriptKind,
    InstallScriptProgress, ModelCandidate, OllamaPhase, PullProgress,
};

pub const EVENT_LOCAL_MODEL_JOB_CREATED: &str = "local_model_job:created";
pub const EVENT_LOCAL_MODEL_JOB_UPDATED: &str = "local_model_job:updated";
pub const EVENT_LOCAL_MODEL_JOB_LOG: &str = "local_model_job:log";
pub const EVENT_LOCAL_MODEL_JOB_COMPLETED: &str = "local_model_job:completed";

const MAX_LOG_LINES_PER_JOB: i64 = 500;

pub type ChatCompletionHook = Arc<dyn Fn(String, String) + Send + Sync + 'static>;

/// Build a `ChatCompletionHook` that rebuilds the desktop's active `AssistantAgent`
/// from the freshly-installed local provider. Both Tauri shell and HTTP server
/// hold the same `Arc<Mutex<Option<AssistantAgent>>>` (it lives in `ha-core::AppState`),
/// so the rebuild logic stays here rather than being copied into each shim.
pub fn rebuild_active_agent_hook(
    agent_cell: Arc<tokio::sync::Mutex<Option<AssistantAgent>>>,
) -> ChatCompletionHook {
    Arc::new(move |provider_id, model_id| {
        let agent_cell = agent_cell.clone();
        tokio::spawn(async move {
            let provider = crate::config::cached_config()
                .providers
                .iter()
                .find(|p| p.id == provider_id)
                .cloned();
            let Some(provider) = provider else {
                crate::app_warn!(
                    "local_model_jobs",
                    "completion_hook",
                    "Provider not found after local model job completion: {}",
                    provider_id
                );
                return;
            };
            let agent = AssistantAgent::new_from_provider(&provider, &model_id);
            *agent_cell.lock().await = Some(agent);
        });
    })
}

static LOCAL_MODEL_JOBS_DB: OnceLock<Arc<LocalModelJobsDB>> = OnceLock::new();
static CANCELS: LazyLock<Mutex<HashMap<String, CancellationToken>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalModelJobKind {
    ChatModel,
    EmbeddingModel,
}

impl LocalModelJobKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChatModel => "chat_model",
            Self::EmbeddingModel => "embedding_model",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "chat_model" => Some(Self::ChatModel),
            "embedding_model" => Some(Self::EmbeddingModel),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalModelJobStatus {
    Running,
    Cancelling,
    Completed,
    Failed,
    Interrupted,
    Cancelled,
}

impl LocalModelJobStatus {
    pub const TERMINAL_SQL_LIST: &'static str = "'completed','failed','interrupted','cancelled'";

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
            Self::Cancelled => "cancelled",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "running" => Some(Self::Running),
            "cancelling" => Some(Self::Cancelling),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "interrupted" => Some(Self::Interrupted),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running | Self::Cancelling)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelJobSnapshot {
    pub job_id: String,
    pub kind: LocalModelJobKind,
    pub model_id: String,
    pub display_name: String,
    pub status: LocalModelJobStatus,
    pub phase: String,
    pub percent: Option<u8>,
    pub error: Option<String>,
    pub result_json: Option<Value>,
    pub created_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelJobLogEntry {
    pub job_id: String,
    pub seq: i64,
    pub kind: String,
    pub message: String,
    pub created_at: i64,
}

pub struct LocalModelJobsDB {
    conn: Mutex<Connection>,
}

impl LocalModelJobsDB {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open local model jobs DB at {}", path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS local_model_jobs (
                job_id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                model_id TEXT NOT NULL,
                display_name TEXT NOT NULL,
                status TEXT NOT NULL,
                phase TEXT NOT NULL,
                percent INTEGER,
                error TEXT,
                result_json TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                completed_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_local_model_jobs_status
                ON local_model_jobs(status, created_at);

            CREATE TABLE IF NOT EXISTS local_model_job_logs (
                job_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                kind TEXT NOT NULL,
                message TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY(job_id, seq)
            );
            CREATE INDEX IF NOT EXISTS idx_local_model_job_logs_job_seq
                ON local_model_job_logs(job_id, seq);",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn insert_job(&self, job: &LocalModelJobSnapshot) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "INSERT INTO local_model_jobs (
                job_id, kind, model_id, display_name, status, phase, percent,
                error, result_json, created_at, updated_at, completed_at
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                job.job_id,
                job.kind.as_str(),
                job.model_id,
                job.display_name,
                job.status.as_str(),
                job.phase,
                job.percent.map(i64::from),
                job.error,
                job.result_json.as_ref().map(Value::to_string),
                job.created_at,
                job.updated_at,
                job.completed_at,
            ],
        )?;
        Ok(())
    }

    fn update_progress(
        &self,
        job_id: &str,
        status: LocalModelJobStatus,
        phase: &str,
        percent: Option<u8>,
        error: Option<&str>,
        result_json: Option<&Value>,
        completed_at: Option<i64>,
    ) -> Result<Option<LocalModelJobSnapshot>> {
        let now = now_secs();
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "UPDATE local_model_jobs
                SET status=?1, phase=?2, percent=?3, error=?4,
                    result_json=COALESCE(?5, result_json),
                    updated_at=?6, completed_at=?7
              WHERE job_id=?8",
            params![
                status.as_str(),
                phase,
                percent.map(i64::from),
                error,
                result_json.map(Value::to_string),
                now,
                completed_at,
                job_id,
            ],
        )?;
        drop(conn);
        self.load(job_id)
    }

    fn mark_interrupted_running(&self) -> Result<Vec<LocalModelJobSnapshot>> {
        let now = now_secs();
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let changed_ids = {
            let mut stmt = conn.prepare(
                "SELECT job_id
                   FROM local_model_jobs
                  WHERE status IN ('running','cancelling')",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            ids
        };
        if changed_ids.is_empty() {
            return Ok(Vec::new());
        }
        conn.execute(
            "UPDATE local_model_jobs
                SET status='interrupted',
                    phase='interrupted',
                    error='Interrupted by application restart',
                    updated_at=?1,
                    completed_at=?1
              WHERE status IN ('running','cancelling')",
            params![now],
        )?;
        drop(conn);
        let mut snapshots = Vec::new();
        for id in changed_ids {
            if let Some(job) = self.load(&id)? {
                snapshots.push(job);
            }
        }
        Ok(snapshots)
    }

    fn mark_cancelling(&self, job_id: &str) -> Result<Option<LocalModelJobSnapshot>> {
        let now = now_secs();
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "UPDATE local_model_jobs
                SET status='cancelling', phase='cancelling', updated_at=?1
              WHERE job_id=?2 AND status IN ('running','cancelling')",
            params![now, job_id],
        )?;
        drop(conn);
        self.load(job_id)
    }

    fn load(&self, job_id: &str) -> Result<Option<LocalModelJobSnapshot>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let result = conn
            .prepare(
                "SELECT job_id, kind, model_id, display_name, status, phase, percent,
                    error, result_json, created_at, updated_at, completed_at
               FROM local_model_jobs
              WHERE job_id=?1",
            )?
            .query_row(params![job_id], row_to_job)
            .optional()
            .map_err(Into::into);
        result
    }

    fn list(&self) -> Result<Vec<LocalModelJobSnapshot>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn.prepare(
            "SELECT job_id, kind, model_id, display_name, status, phase, percent,
                    error, result_json, created_at, updated_at, completed_at
               FROM local_model_jobs
              ORDER BY created_at DESC
              LIMIT 100",
        )?;
        let rows = stmt.query_map([], row_to_job)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn insert_log(&self, job_id: &str, kind: &str, message: &str) -> Result<LocalModelJobLogEntry> {
        let now = now_secs();
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM local_model_job_logs WHERE job_id=?1",
            params![job_id],
            |row| row.get(0),
        )?;
        conn.execute(
            "INSERT INTO local_model_job_logs(job_id, seq, kind, message, created_at)
             VALUES (?1,?2,?3,?4,?5)",
            params![job_id, seq, kind, message, now],
        )?;
        conn.execute(
            "DELETE FROM local_model_job_logs
              WHERE job_id=?1
                AND seq <= (
                    SELECT COALESCE(MAX(seq), 0) - ?2
                      FROM local_model_job_logs
                     WHERE job_id=?1
                )",
            params![job_id, MAX_LOG_LINES_PER_JOB],
        )?;
        Ok(LocalModelJobLogEntry {
            job_id: job_id.to_string(),
            seq,
            kind: kind.to_string(),
            message: message.to_string(),
            created_at: now,
        })
    }

    fn logs(&self, job_id: &str, after_seq: Option<i64>) -> Result<Vec<LocalModelJobLogEntry>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn.prepare(
            "SELECT job_id, seq, kind, message, created_at
               FROM local_model_job_logs
              WHERE job_id=?1 AND seq > ?2
              ORDER BY seq ASC
              LIMIT 500",
        )?;
        let rows = stmt.query_map(params![job_id, after_seq.unwrap_or(0)], |row| {
            Ok(LocalModelJobLogEntry {
                job_id: row.get(0)?,
                seq: row.get(1)?,
                kind: row.get(2)?,
                message: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn clear(&self, job_id: &str) -> Result<()> {
        let job = self
            .load(job_id)?
            .ok_or_else(|| anyhow!("Local model job not found: {job_id}"))?;
        if !job.status.is_terminal() {
            return Err(anyhow!("Only terminal jobs can be cleared"));
        }
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "DELETE FROM local_model_job_logs WHERE job_id=?1",
            params![job_id],
        )?;
        conn.execute(
            "DELETE FROM local_model_jobs WHERE job_id=?1",
            params![job_id],
        )?;
        Ok(())
    }
}

fn row_to_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<LocalModelJobSnapshot> {
    let kind_raw: String = row.get(1)?;
    let status_raw: String = row.get(4)?;
    let result_raw: Option<String> = row.get(8)?;
    let percent_raw: Option<i64> = row.get(6)?;
    let kind = LocalModelJobKind::parse(&kind_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            format!("unknown local model job kind: {kind_raw}").into(),
        )
    })?;
    let status = LocalModelJobStatus::parse(&status_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            format!("unknown local model job status: {status_raw}").into(),
        )
    })?;
    Ok(LocalModelJobSnapshot {
        job_id: row.get(0)?,
        kind,
        model_id: row.get(2)?,
        display_name: row.get(3)?,
        status,
        phase: row.get(5)?,
        percent: percent_raw.and_then(|n| u8::try_from(n).ok()),
        error: row.get(7)?,
        result_json: result_raw.and_then(|raw| serde_json::from_str(&raw).ok()),
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        completed_at: row.get(11)?,
    })
}

pub fn set_local_model_jobs_db(db: Arc<LocalModelJobsDB>) {
    let _ = LOCAL_MODEL_JOBS_DB.set(db);
}

pub fn get_local_model_jobs_db() -> Option<&'static Arc<LocalModelJobsDB>> {
    LOCAL_MODEL_JOBS_DB.get()
}

pub fn replay_interrupted_jobs() {
    let Some(db) = get_local_model_jobs_db() else {
        return;
    };
    match db.mark_interrupted_running() {
        Ok(rows) => {
            for job in rows
                .into_iter()
                .filter(|job| job.status == LocalModelJobStatus::Interrupted)
            {
                emit_snapshot(EVENT_LOCAL_MODEL_JOB_COMPLETED, &job);
            }
        }
        Err(e) => app_warn!(
            "local_model_jobs",
            "replay",
            "Failed to mark running jobs interrupted: {}",
            e
        ),
    }
}

pub fn list_jobs() -> Result<Vec<LocalModelJobSnapshot>> {
    require_db()?.list()
}

pub fn get_job(job_id: &str) -> Result<Option<LocalModelJobSnapshot>> {
    require_db()?.load(job_id)
}

pub fn get_logs(job_id: &str, after_seq: Option<i64>) -> Result<Vec<LocalModelJobLogEntry>> {
    require_db()?.logs(job_id, after_seq)
}

pub fn clear_job(job_id: &str) -> Result<()> {
    require_db()?.clear(job_id)
}

pub fn cancel_job(job_id: &str) -> Result<LocalModelJobSnapshot> {
    let db = require_db()?;
    let job = db
        .load(job_id)?
        .ok_or_else(|| anyhow!("Local model job not found: {job_id}"))?;
    if job.status.is_terminal() {
        return Ok(job);
    }
    if let Some(token) = CANCELS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(job_id)
        .cloned()
    {
        token.cancel();
    }
    let snapshot = db
        .mark_cancelling(job_id)?
        .ok_or_else(|| anyhow!("Local model job not found: {job_id}"))?;
    crate::app_info!(
        "local_model_jobs",
        "cancel",
        "Local model job cancel requested: {} ({} {})",
        job_id,
        snapshot.kind.as_str(),
        snapshot.model_id
    );
    emit_snapshot(EVENT_LOCAL_MODEL_JOB_UPDATED, &snapshot);
    Ok(snapshot)
}

pub fn start_chat_model_job(
    model: ModelCandidate,
    on_complete: Option<ChatCompletionHook>,
) -> Result<LocalModelJobSnapshot> {
    let model_id = model.id.clone();
    let display_name = model.display_name.clone();
    spawn_job(
        LocalModelJobKind::ChatModel,
        model_id,
        display_name,
        move |job_id, token| run_chat_model_job(job_id, model, token, on_complete),
    )
}

pub fn start_embedding_job(model: OllamaEmbeddingModel) -> Result<LocalModelJobSnapshot> {
    let model_id = model.id.clone();
    let display_name = model.display_name.clone();
    spawn_job(
        LocalModelJobKind::EmbeddingModel,
        model_id,
        display_name,
        move |job_id, token| run_embedding_job(job_id, model, token),
    )
}

pub fn retry_job(
    job_id: &str,
    on_chat_complete: Option<ChatCompletionHook>,
) -> Result<LocalModelJobSnapshot> {
    let job = require_db()?
        .load(job_id)?
        .ok_or_else(|| anyhow!("Local model job not found: {job_id}"))?;
    if !job.status.is_terminal() {
        return Err(anyhow!("Only terminal jobs can be retried"));
    }
    match job.kind {
        LocalModelJobKind::ChatModel => {
            let model = local_llm::model_catalog()
                .into_iter()
                .find(|model| model.id == job.model_id)
                .ok_or_else(|| anyhow!("Unsupported Ollama model: {}", job.model_id))?;
            start_chat_model_job(model, on_chat_complete)
        }
        LocalModelJobKind::EmbeddingModel => {
            let model = local_embedding::resolve_catalog_model(&job.model_id)?;
            start_embedding_job(model)
        }
    }
}

fn spawn_job<F, Fut>(
    kind: LocalModelJobKind,
    model_id: String,
    display_name: String,
    runner: F,
) -> Result<LocalModelJobSnapshot>
where
    F: FnOnce(String, CancellationToken) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let db = require_db()?.clone();
    let job_id = format!("lmjob_{}", uuid::Uuid::new_v4().simple());
    let now = now_secs();
    let snapshot = LocalModelJobSnapshot {
        job_id: job_id.clone(),
        kind,
        model_id,
        display_name,
        status: LocalModelJobStatus::Running,
        phase: "queued".into(),
        percent: Some(0),
        error: None,
        result_json: None,
        created_at: now,
        updated_at: now,
        completed_at: None,
    };
    db.insert_job(&snapshot)?;
    crate::app_info!(
        "local_model_jobs",
        "spawn",
        "Local model job started: {} ({} {})",
        snapshot.job_id,
        snapshot.kind.as_str(),
        snapshot.model_id
    );
    emit_snapshot(EVENT_LOCAL_MODEL_JOB_CREATED, &snapshot);

    let token = CancellationToken::new();
    CANCELS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(job_id.clone(), token.clone());

    let job_id_for_task = job_id.clone();
    tokio::spawn(async move {
        runner(job_id_for_task.clone(), token).await;
        CANCELS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&job_id_for_task);
    });

    Ok(snapshot)
}

async fn run_chat_model_job(
    job_id: String,
    model: ModelCandidate,
    cancel_token: CancellationToken,
    on_complete: Option<ChatCompletionHook>,
) {
    let final_result = match run_common_setup(&job_id, &cancel_token).await {
        Ok(()) => {
            let throttle = Arc::new(Mutex::new(ProgressThrottle::default()));
            let job_id_for_progress = job_id.clone();
            match local_llm::pull_and_activate_cancellable(
                model,
                move |progress| handle_pull_progress(&job_id_for_progress, progress, &throttle),
                cancel_token.clone(),
            )
            .await
            {
                Ok((provider_id, model_id)) => {
                    if let Some(hook) = on_complete {
                        hook(provider_id.clone(), model_id.clone());
                    }
                    Ok(json!({ "providerId": provider_id, "modelId": model_id }))
                }
                Err(e) => Err(e),
            }
        }
        Err(e) => Err(e),
    };
    finish_job(&job_id, final_result, &cancel_token);
}

async fn run_embedding_job(
    job_id: String,
    model: OllamaEmbeddingModel,
    cancel_token: CancellationToken,
) {
    let final_result = match run_common_setup(&job_id, &cancel_token).await {
        Ok(()) => {
            let throttle = Arc::new(Mutex::new(ProgressThrottle::default()));
            let job_id_for_progress = job_id.clone();
            local_embedding::pull_and_activate_cancellable(
                model,
                move |progress| handle_pull_progress(&job_id_for_progress, progress, &throttle),
                cancel_token.clone(),
            )
            .await
            .map(|config| json!(config))
        }
        Err(e) => Err(e),
    };

    finish_job(&job_id, final_result, &cancel_token);
}

async fn run_common_setup(job_id: &str, cancel_token: &CancellationToken) -> Result<()> {
    update_job(
        job_id,
        LocalModelJobStatus::Running,
        "checking-ollama",
        Some(0),
        None,
        None,
    );
    let mut status = local_llm::detect_ollama().await;
    if cancel_token.is_cancelled() {
        return Err(anyhow!("Local model job was cancelled"));
    }

    if status.phase == OllamaPhase::NotInstalled {
        append_log(job_id, "step", "Install Ollama");
        update_job(
            job_id,
            LocalModelJobStatus::Running,
            "install-ollama",
            Some(0),
            None,
            None,
        );
        let job_id_for_progress = job_id.to_string();
        install_ollama_via_script_cancellable(
            move |progress| handle_install_progress(&job_id_for_progress, progress),
            cancel_token.clone(),
        )
        .await?;
        status = local_llm::detect_ollama().await;
    }

    if status.phase != OllamaPhase::Running {
        append_log(job_id, "step", "Start Ollama");
        update_job(
            job_id,
            LocalModelJobStatus::Running,
            "start-ollama",
            Some(5),
            None,
            None,
        );
        tokio::select! {
            result = start_ollama() => result?,
            _ = cancel_token.cancelled() => return Err(anyhow!("Local model job was cancelled")),
        }
    }

    Ok(())
}

fn handle_install_progress(job_id: &str, progress: &InstallScriptProgress) {
    match progress.kind {
        InstallScriptKind::Step => {
            update_job(
                job_id,
                LocalModelJobStatus::Running,
                &progress.message,
                None,
                None,
                None,
            );
            append_log(job_id, "step", &progress.message);
        }
        InstallScriptKind::Log => append_log(job_id, "log", &progress.message),
        InstallScriptKind::Error => {
            append_log(job_id, "error", &progress.message);
            update_job(
                job_id,
                LocalModelJobStatus::Running,
                "install-ollama",
                None,
                Some(progress.message.clone()),
                None,
            );
        }
    }
}

#[derive(Default)]
struct ProgressThrottle {
    last_emit: Option<Instant>,
    last_phase: Option<String>,
    last_percent: Option<u8>,
}

impl ProgressThrottle {
    fn should_emit(&mut self, phase: &str, percent: Option<u8>) -> bool {
        let now = Instant::now();
        let phase_changed = self.last_phase.as_deref() != Some(phase);
        let terminal = matches!(percent, Some(100)) || phase.eq_ignore_ascii_case("success");
        let percent_changed = match (self.last_percent, percent) {
            (Some(a), Some(b)) => a != b,
            (None, Some(_)) | (Some(_), None) => true,
            (None, None) => false,
        };
        let due = self
            .last_emit
            .map(|t| now.duration_since(t).as_millis() >= PROGRESS_THROTTLE_MS)
            .unwrap_or(true);
        if phase_changed || terminal || (percent_changed && due) {
            self.last_emit = Some(now);
            self.last_phase = Some(phase.to_string());
            self.last_percent = percent;
            true
        } else {
            false
        }
    }
}

fn handle_pull_progress(
    job_id: &str,
    progress: &PullProgress,
    throttle: &Arc<Mutex<ProgressThrottle>>,
) {
    {
        let mut guard = throttle.lock().unwrap_or_else(|p| p.into_inner());
        if !guard.should_emit(&progress.phase, progress.percent) {
            return;
        }
    }
    update_job(
        job_id,
        LocalModelJobStatus::Running,
        &progress.phase,
        progress.percent,
        None,
        None,
    );
    let suffix = progress
        .percent
        .map(|p| format!(" {p}%"))
        .unwrap_or_default();
    append_log(job_id, "log", &format!("{}{}", progress.phase, suffix));
}

fn finish_job(job_id: &str, result: Result<Value>, cancel_token: &CancellationToken) {
    let job_before = get_job(job_id).ok().flatten();
    let status_before = job_before.as_ref().map(|job| job.status);
    let cancelled = cancel_token.is_cancelled()
        || matches!(status_before, Some(LocalModelJobStatus::Cancelling));
    let final_status = if cancelled {
        LocalModelJobStatus::Cancelled
    } else if result.is_ok() {
        LocalModelJobStatus::Completed
    } else {
        LocalModelJobStatus::Failed
    };
    let (phase, error, result_json) = match result {
        Ok(value) => ("done".to_string(), None, Some(value)),
        Err(e) => {
            let msg = e.to_string();
            append_log(job_id, "error", &msg);
            // Keep the phase that was active when the job stopped so the UI
            // can still show *where* it failed; the status badge tells the user
            // *that* it failed.
            let last_phase = job_before
                .as_ref()
                .map(|job| job.phase.clone())
                .unwrap_or_default();
            (last_phase, Some(msg), None)
        }
    };
    update_job(
        job_id,
        final_status,
        &phase,
        if final_status == LocalModelJobStatus::Completed {
            Some(100)
        } else {
            None
        },
        error,
        result_json,
    );
}

fn update_job(
    job_id: &str,
    status: LocalModelJobStatus,
    phase: &str,
    percent: Option<u8>,
    error: Option<String>,
    result_json: Option<Value>,
) {
    let Some(db) = get_local_model_jobs_db() else {
        return;
    };
    let completed_at = if status.is_terminal() {
        Some(now_secs())
    } else {
        None
    };
    match db.update_progress(
        job_id,
        status,
        phase,
        percent,
        error.as_deref(),
        result_json.as_ref(),
        completed_at,
    ) {
        Ok(Some(snapshot)) => {
            emit_snapshot(EVENT_LOCAL_MODEL_JOB_UPDATED, &snapshot);
            if snapshot.status.is_terminal() {
                crate::app_info!(
                    "local_model_jobs",
                    "finish",
                    "Local model job finished: {} status={} kind={} model={}",
                    snapshot.job_id,
                    snapshot.status.as_str(),
                    snapshot.kind.as_str(),
                    snapshot.model_id
                );
                emit_snapshot(EVENT_LOCAL_MODEL_JOB_COMPLETED, &snapshot);
            }
        }
        Ok(None) => {}
        Err(e) => app_warn!(
            "local_model_jobs",
            "update",
            "Failed to update local model job {}: {}",
            job_id,
            e
        ),
    }
}

fn append_log(job_id: &str, kind: &str, message: &str) {
    let Some(db) = get_local_model_jobs_db() else {
        return;
    };
    match db.insert_log(job_id, kind, message) {
        Ok(entry) => {
            if let Some(bus) = crate::get_event_bus() {
                bus.emit(EVENT_LOCAL_MODEL_JOB_LOG, json!(entry));
            }
        }
        Err(e) => app_warn!(
            "local_model_jobs",
            "log",
            "Failed to append local model job log {}: {}",
            job_id,
            e
        ),
    }
}

fn emit_snapshot(event: &str, snapshot: &LocalModelJobSnapshot) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(event, json!(snapshot));
    }
}

fn require_db() -> Result<&'static Arc<LocalModelJobsDB>> {
    get_local_model_jobs_db().ok_or_else(|| anyhow!("Local model jobs DB is not initialized"))
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_job() -> LocalModelJobSnapshot {
        LocalModelJobSnapshot {
            job_id: "lmjob_test".into(),
            kind: LocalModelJobKind::ChatModel,
            model_id: "gemma4:e2b".into(),
            display_name: "Gemma".into(),
            status: LocalModelJobStatus::Running,
            phase: "queued".into(),
            percent: Some(0),
            error: None,
            result_json: None,
            created_at: now_secs(),
            updated_at: now_secs(),
            completed_at: None,
        }
    }

    #[test]
    fn db_crud_logs_and_replay() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = LocalModelJobsDB::open(&tmp.path().join("jobs.db")).expect("db");
        let job = sample_job();
        db.insert_job(&job).expect("insert");

        let loaded = db.load(&job.job_id).expect("load").expect("job");
        assert_eq!(loaded.status, LocalModelJobStatus::Running);

        db.insert_log(&job.job_id, "log", "hello").expect("log");
        assert_eq!(db.logs(&job.job_id, None).expect("logs").len(), 1);

        db.mark_interrupted_running().expect("interrupt");
        let interrupted = db.load(&job.job_id).expect("load").expect("job");
        assert_eq!(interrupted.status, LocalModelJobStatus::Interrupted);

        db.clear(&job.job_id).expect("clear");
        assert!(db.load(&job.job_id).expect("load").is_none());
    }
}
