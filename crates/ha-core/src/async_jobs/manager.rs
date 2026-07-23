//! `JobManager` — the single entry point for background jobs (R1).
//!
//! Every production code path that spawns / cancels / lists / inspects /
//! recovers a background job goes through this facade rather than the loose
//! module functions. Today only [`JobKind::Tool`] is wired for execution;
//! `Subagent` (R6) and `Group` (R5) will extend `JobManager` with their own
//! spawn entries instead of inventing parallel subsystem APIs — unifying every
//! kind behind one type (one lifecycle, one persistence, one cancel/list/inject
//! surface) is the whole point of the R1 model.
//!
//! The facade is a thin delegator over the proven `async_jobs` internals (the
//! Tool executor): spawn → [`spawn`](super::spawn), cancel/cleanup/replay →
//! [`mod`](super), scheduling → [`spawn::run_scheduler`]. It owns no state; the
//! DB handle stays a process-global [`OnceLock`](super::ASYNC_JOBS_DB) set at
//! init. Reads route through `get` / `list_active_by_session`; the raw DB stays
//! `pub(crate)` for bootstrap (`app_init`) and white-box tests only.

use anyhow::{anyhow, Result};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

use super::types::{BackgroundJob, BackgroundJobSnapshot, JobKind, JobOrigin, JobStatus};
use crate::subagent::SubagentStatus;
use crate::tools::ToolExecContext;

const TERMINAL_WAIT_INITIAL_BACKOFF: Duration = Duration::from_millis(100);
const TERMINAL_WAIT_MAX_BACKOFF: Duration = Duration::from_secs(2);

struct WaiterGuard {
    job_id: String,
    notify: Arc<Notify>,
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        super::wait::cleanup_if_last_waiter(&self.job_id, &self.notify);
    }
}

/// Per-item outcome counts for a finished knowledge source import run,
/// carried on the `job:completed` payload so `useDesktopAlerts` can build a
/// real "imported N, failed M" notification instead of showing the raw tool
/// name. Plain `u32`s so `async_jobs` doesn't need to depend on `knowledge`
/// types — the caller (`knowledge::source`) maps its own richer run/item
/// model down to this before calling `JobManager::finish_knowledge_import`.
pub struct KnowledgeImportCounts {
    pub imported: u32,
    pub duplicate: u32,
    pub failed: u32,
    pub total: u32,
}

/// Single entry point for background-job operations (R1). Zero-sized; all
/// methods are associated functions delegating to the shared internals.
pub struct JobManager;

impl JobManager {
    // ── Spawn (Tool executor — the only wired kind in R1) ──────────────────

    /// Spawn an explicit background tool job (`run_in_background: true` or an
    /// agent `always-background` policy). Returns the synthetic started result
    /// JSON the model receives immediately. R6/R5 add `spawn_subagent` /
    /// `spawn_group` here.
    pub fn spawn_tool(
        tool_name: &str,
        args: Value,
        ctx: ToolExecContext,
        origin: JobOrigin,
    ) -> Result<String> {
        super::spawn::spawn_explicit_job(tool_name, args, ctx, origin)
    }

    /// Allocate a background job id before launching the side effect, so a
    /// durable parent operation can persist the child handle first.
    pub fn new_job_id() -> String {
        super::spawn::new_job_id()
    }

    /// Register a long-lived Loop monitor in the unified background-job
    /// lifecycle. The Loop adapter owns the actual watcher/task and calls
    /// [`Self::finish_monitor`] on terminal outcomes.
    pub fn register_monitor(
        session_id: &str,
        watch_id: &str,
        adapter: &str,
        spec: &Value,
    ) -> Result<Option<String>> {
        let Some(db) = super::get_async_jobs_db() else {
            return Ok(None);
        };
        let job_id = Self::new_job_id();
        let now = chrono::Utc::now().timestamp();
        let job = BackgroundJob {
            job_id: job_id.clone(),
            kind: JobKind::Monitor,
            subagent_run_id: None,
            group_id: None,
            session_id: Some(session_id.to_string()),
            agent_id: None,
            tool_name: format!("loop_monitor:{adapter}"),
            tool_call_id: Some(watch_id.to_string()),
            args_json: serde_json::to_string(spec)?,
            status: JobStatus::Running,
            result_preview: None,
            result_path: None,
            error: None,
            created_at: now,
            completed_at: None,
            injected: true,
            origin: "loop_monitor".to_string(),
            approval_origin: Some("policy_allow".to_string()),
            incognito: false,
            pid: None,
            cancel_requested: false,
        };
        db.insert(&job)?;
        super::events::emit_created(
            &job_id,
            JobKind::Monitor,
            &job.tool_name,
            JobStatus::Running.as_str(),
            Some(session_id),
        );
        Ok(Some(job_id))
    }

    pub fn finish_monitor(
        job_id: &str,
        status: JobStatus,
        result_preview: Option<&str>,
        error: Option<&str>,
    ) -> Result<bool> {
        if !status.is_terminal() {
            return Err(anyhow!("monitor terminal status required"));
        }
        let Some(db) = super::get_async_jobs_db() else {
            return Ok(false);
        };
        let Some(job) = db.load(job_id)? else {
            return Ok(false);
        };
        if job.kind != JobKind::Monitor {
            return Err(anyhow!("background job {job_id} is not a monitor"));
        }
        let changed = db.update_terminal(
            job_id,
            status,
            result_preview,
            None,
            error,
            chrono::Utc::now().timestamp(),
        )?;
        if changed {
            super::events::emit_completed(
                job_id,
                JobKind::Monitor,
                &job.tool_name,
                status.as_str(),
                job.session_id.as_deref(),
            );
        }
        Ok(changed)
    }

    /// Spawn an explicit background tool job using a caller-preallocated id.
    /// This is for durable parents that must record the child handle before the
    /// side effect starts; ordinary tool calls should use [`Self::spawn_tool`].
    pub fn spawn_tool_with_id(
        tool_name: &str,
        args: Value,
        ctx: ToolExecContext,
        origin: JobOrigin,
        job_id: String,
    ) -> Result<String> {
        super::spawn::spawn_explicit_job_with_id(tool_name, args, ctx, origin, job_id)
    }

    /// Run a tool synchronously but auto-background it if it exceeds the budget
    /// (`config.async_tools.auto_background_secs`). Returns the inline result if
    /// it finished in time, else the synthetic started result.
    pub async fn dispatch_tool_with_auto_background(
        name: &str,
        args: &Value,
        ctx: &ToolExecContext,
        auto_bg_secs: u64,
    ) -> Result<String> {
        super::spawn::dispatch_with_auto_background(name, args, ctx, auto_bg_secs).await
    }

    // ── Reads (any kind) ───────────────────────────────────────────────────

    /// Load a single job snapshot by id. `Ok(None)` if the job is unknown or the
    /// DB is not initialized.
    pub fn get(job_id: &str) -> Result<Option<BackgroundJob>> {
        match super::get_async_jobs_db() {
            Some(db) => db.load(job_id),
            None => Ok(None),
        }
    }

    /// Wait until a job reaches a terminal state. Unlike the model-facing
    /// `job_status(wait)`, this helper is for host runtimes that genuinely own
    /// the child work and need the result before completing their parent op.
    pub async fn wait_for_terminal(
        job_id: &str,
        timeout: Option<Duration>,
    ) -> Result<Option<BackgroundJob>> {
        // Each poll is a synchronous JobsDB read — route every one through the
        // blocking pool so awaiting a long job never pins an async worker.
        async fn load_off_worker(
            db: &std::sync::Arc<super::db::JobsDB>,
            job_id: &str,
        ) -> Result<Option<BackgroundJob>> {
            let db = db.clone();
            let job_id = job_id.to_string();
            crate::blocking::run_blocking(move || db.load(&job_id)).await
        }

        let db = super::get_async_jobs_db()
            .ok_or_else(|| anyhow!("Async jobs DB not initialized"))?
            .clone();
        let Some(initial) = load_off_worker(&db, job_id).await? else {
            return Ok(None);
        };
        if initial.status.is_terminal() {
            return Ok(Some(initial));
        }

        let guard = WaiterGuard {
            job_id: job_id.to_string(),
            notify: super::wait::register_waiter(job_id),
        };

        let Some(recheck) = load_off_worker(&db, job_id).await? else {
            return Ok(None);
        };
        if recheck.status.is_terminal() {
            return Ok(Some(recheck));
        }

        let deadline = timeout.map(|duration| std::time::Instant::now() + duration);
        let mut backoff = TERMINAL_WAIT_INITIAL_BACKOFF;
        loop {
            let sleep_dur = match deadline {
                Some(deadline) => {
                    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                    if remaining.is_zero() {
                        return load_off_worker(&db, job_id).await;
                    }
                    std::cmp::min(backoff, remaining)
                }
                None => backoff,
            };

            tokio::select! {
                _ = guard.notify.notified() => {}
                _ = tokio::time::sleep(sleep_dur) => {
                    backoff = std::cmp::min(
                        backoff.saturating_mul(3) / 2,
                        TERMINAL_WAIT_MAX_BACKOFF,
                    );
                }
            }

            let Some(job) = load_off_worker(&db, job_id).await? else {
                return Ok(None);
            };
            if job.status.is_terminal() {
                return Ok(Some(job));
            }
        }
    }

    /// All active (`queued`/`running`/`cancelling`/`awaiting_approval`) jobs
    /// owned by a session. Empty when the DB is not initialized.
    pub fn list_active_by_session(session_id: &str) -> Result<Vec<BackgroundJob>> {
        match super::get_async_jobs_db() {
            Some(db) => db.list_active_by_session(session_id),
            None => Ok(Vec::new()),
        }
    }

    /// Bounded active-job read for status projections. Lifecycle owners must
    /// use [`Self::list_active_by_session`] to avoid skipping cleanup targets.
    pub fn list_active_by_session_limited(
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<BackgroundJob>> {
        match super::get_async_jobs_db() {
            Some(db) => db.list_active_by_session_limited(session_id, limit),
            None => Ok(Vec::new()),
        }
    }

    /// Active executable work, excluding long-lived monitor subscriptions.
    /// Goal Runner uses this to avoid treating a healthy watcher as a child job
    /// that must terminate before the Goal can continue.
    pub fn list_active_work_by_session(session_id: &str) -> Result<Vec<BackgroundJob>> {
        Ok(Self::list_active_by_session(session_id)?
            .into_iter()
            .filter(|job| job.kind != JobKind::Monitor)
            .collect())
    }

    // ── Owner-plane snapshots (R4 panel) ───────────────────────────────────
    //
    // The desktop / HTTP owner plane (host-trusted) reads these to render the
    // background-jobs panel + header badge. They are deliberately separate from
    // the model-facing `job_status` JSON: camelCase, display-oriented, no
    // agent-steering hints. Group child projections are folded out (the panel
    // shows the `Group` row's N-of-M progress, not its N child rows).

    /// A session's background jobs for the R4 panel — active jobs plus recent
    /// terminal ones, active-first/newest-first (see [`db::JobsDB::list_for_session`]).
    /// Empty when the DB is not initialized.
    pub fn list_session_snapshots(session_id: &str) -> Result<Vec<BackgroundJobSnapshot>> {
        let Some(db) = super::get_async_jobs_db() else {
            return Ok(Vec::new());
        };
        // Active-first ordering (see `list_for_session`) drops terminal rows
        // first; only a session with >PANEL_LIMIT simultaneously-active TOP-LEVEL
        // jobs (running cap + the bounded wait queue) would lose an active row.
        const PANEL_LIMIT: usize = 50;
        let jobs = db.list_for_session(session_id, PANEL_LIMIT)?;
        Ok(jobs
            .iter()
            // Grouped Subagent children are already excluded at the query layer
            // (so the limit budgets only top-level rows and a batch's Group row
            // can't be cut — see `list_for_session`). This filter is a defensive
            // backstop in case that query is ever changed.
            .filter(|j| !(j.kind == JobKind::Subagent && j.group_id.is_some()))
            .map(|j| Self::snapshot_from_job(j, false))
            .collect())
    }

    /// A single job snapshot for the R4 panel, including the live running-output
    /// tail for a backgrounded `exec`. `Ok(None)` when unknown / DB uninitialized.
    pub fn get_job_snapshot(job_id: &str) -> Result<Option<BackgroundJobSnapshot>> {
        let Some(db) = super::get_async_jobs_db() else {
            return Ok(None);
        };
        Ok(db.load(job_id)?.map(|j| Self::snapshot_from_job(&j, true)))
    }

    /// Build the owner-plane snapshot for a row. `include_output_tail` attaches
    /// the live tail for a still-running job (single-job `get` only — never the
    /// list roster).
    fn snapshot_from_job(job: &BackgroundJob, include_output_tail: bool) -> BackgroundJobSnapshot {
        let (child_count, children_terminal, children_completed, children_failed) =
            if job.kind == JobKind::Group {
                match Self::group_progress(&job.job_id) {
                    Some((total, terminal, completed, failed)) => {
                        (Some(total), Some(terminal), Some(completed), Some(failed))
                    }
                    None => (None, None, None, None),
                }
            } else {
                (None, None, None, None)
            };
        let output_tail = if include_output_tail
            && matches!(job.status, JobStatus::Running | JobStatus::Cancelling)
        {
            super::output_tail::read(&job.job_id).filter(|t| !t.is_empty())
        } else {
            None
        };
        BackgroundJobSnapshot {
            job_id: job.job_id.clone(),
            kind: job.kind,
            status: job.status,
            tool: job.tool_name.clone(),
            label: Self::display_label(job),
            origin: job.origin.clone(),
            session_id: job.session_id.clone(),
            created_at: job.created_at,
            completed_at: job.completed_at,
            error: job.error.clone(),
            // Incognito jobs keep an inline preview in the DB row (persist_result
            // skips only the disk spool) — redact it from the owner-plane snapshot
            // so the bare-id `get_background_job` can't surface incognito output
            // (parity with `output_tail` None + args_json redaction for incognito).
            result_preview: if job.incognito {
                None
            } else {
                job.result_preview.clone()
            },
            result_path: if job.incognito {
                None
            } else {
                job.result_path.clone()
            },
            child_count,
            children_terminal,
            children_completed,
            children_failed,
            subagent_run_id: job.subagent_run_id.clone(),
            output_tail,
        }
    }

    /// Concise display label: for a backgrounded `exec`, the command's first line
    /// (truncated); for any other tool, the tool name. Empty for group / subagent
    /// kinds — the frontend localizes those ("任务组 N" / the agent name) and the
    /// projection carries no copyable content anyway.
    fn display_label(job: &BackgroundJob) -> String {
        if job.kind != JobKind::Tool {
            return String::new();
        }
        if job.tool_name == "exec" {
            if let Ok(v) = serde_json::from_str::<Value>(&job.args_json) {
                if let Some(cmd) = v.get("command").and_then(|c| c.as_str()) {
                    let head = cmd
                        .lines()
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or("")
                        .trim();
                    if !head.is_empty() {
                        return crate::truncate_utf8(head, 120).to_string();
                    }
                }
            }
        }
        job.tool_name.clone()
    }

    // ── Cancellation / cleanup ─────────────────────────────────────────────

    /// Best-effort cancel a single job (in-process token + cross-process DB
    /// flag). Returns the updated snapshot, or `None` if the job is unknown.
    pub fn cancel(job_id: &str) -> Result<Option<BackgroundJob>> {
        super::cancel_job(job_id)
    }

    /// Cancel every active job owned by a session (session delete / DELETE-4).
    /// Returns the number cancelled.
    pub fn cancel_for_session(session_id: &str) -> usize {
        super::cancel_jobs_for_session(session_id)
    }

    /// Physically delete all job rows + spool files for a session (incognito
    /// burn-on-close, INCOG-2). Returns the number of rows deleted.
    pub fn purge_for_session(session_id: &str) -> u64 {
        super::purge_jobs_for_session(session_id)
    }

    // ── Recovery / scheduling lifecycle ────────────────────────────────────

    /// Startup recovery (Primary-only): mark interrupted survivors + re-dispatch
    /// terminal-but-uninjected jobs to their parent sessions.
    pub fn replay_pending() {
        super::replay_pending_jobs()
    }

    /// The per-process (tier-agnostic) queue scheduler loop — promotes queued
    /// jobs into freed slots. Idempotent: at most one loop runs per process.
    pub async fn run_scheduler() {
        super::spawn::run_scheduler().await
    }

    /// Spawn the retention sweep loop (Primary-only): purges aged terminal rows
    /// + orphan spool files. No-op ticker if retention is disabled.
    pub fn spawn_retention_loop() {
        super::retention::spawn_background_loop()
    }

    // ── Owner-plane knowledge import projection ───────────────────────────
    //
    // Knowledge source import runs have their own durable fact layer
    // (`knowledge_source_import_runs/items` in sessions.db). The row created
    // here is only the unified background-job lifecycle projection, so owner
    // work benefits from the same restart/interruption accounting without
    // injecting anything into a chat session.

    pub fn spawn_knowledge_import(kb_id: &str, run_id: &str, total_count: u32) -> Option<String> {
        let db = super::get_async_jobs_db()?;
        let job_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();
        let job = BackgroundJob {
            job_id: job_id.clone(),
            kind: JobKind::Tool,
            subagent_run_id: None,
            group_id: None,
            session_id: None,
            agent_id: None,
            tool_name: "knowledge_source_import".to_string(),
            tool_call_id: None,
            args_json: serde_json::json!({
                "kbId": kb_id,
                "runId": run_id,
                "totalCount": total_count,
            })
            .to_string(),
            status: JobStatus::Running,
            result_preview: None,
            result_path: None,
            error: None,
            created_at: now,
            completed_at: None,
            injected: true,
            origin: "owner".to_string(),
            approval_origin: None,
            incognito: false,
            pid: None,
            cancel_requested: false,
        };
        if let Err(e) = db.insert(&job) {
            crate::app_warn!(
                "async_jobs",
                "knowledge_import",
                "Failed to insert knowledge import job for run {}: {}",
                run_id,
                e
            );
            return None;
        }
        super::events::emit_created(
            &job_id,
            JobKind::Tool,
            "knowledge_source_import",
            JobStatus::Running.as_str(),
            None,
        );
        Some(job_id)
    }

    /// Push a per-item progress tick for a running knowledge import job
    /// (`current` of `total` items settled). First real `Tool`-kind caller of
    /// `events::emit_progress` — previously only `Group` fan-out used it.
    /// A batch is capped at `MAX_SOURCE_IMPORT_BATCH_ITEMS` items and this is
    /// called at most twice per item, so unlike `local_model_jobs` progress
    /// there's no need for throttling here.
    pub fn progress_knowledge_import(job_id: &str, current: usize, total: usize) {
        super::events::emit_progress(job_id, JobKind::Tool, None, current, total);
    }

    pub fn finish_knowledge_import(
        job_id: &str,
        status: JobStatus,
        result_preview: Option<&str>,
        error: Option<&str>,
        counts: Option<KnowledgeImportCounts>,
    ) {
        let Some(db) = super::get_async_jobs_db() else {
            return;
        };
        let now = chrono::Utc::now().timestamp();
        if let Err(e) = db.update_terminal(job_id, status, result_preview, None, error, now) {
            crate::app_warn!(
                "async_jobs",
                "knowledge_import",
                "Failed to finish knowledge import job {}: {}",
                job_id,
                e
            );
            return;
        }
        let _ = db.mark_injected(job_id);
        // `emit_completed`'s generic payload (shared by ~6 other Tool/Group/
        // Subagent call sites) has no room for structured counts, so this one
        // call site builds its own payload directly instead of widening that
        // shared signature. Without these counts `useDesktopAlerts` has
        // nothing to build a real "imported N, failed M" notification from —
        // only the raw tool name.
        if let Some(bus) = crate::get_event_bus() {
            let mut payload = serde_json::json!({
                "job_id": job_id,
                "kind": JobKind::Tool.as_str(),
                "tool": "knowledge_source_import",
                "status": status.as_str(),
                "session_id": serde_json::Value::Null,
            });
            if let (Some(obj), Some(c)) = (payload.as_object_mut(), counts) {
                obj.insert("imported_count".into(), serde_json::json!(c.imported));
                obj.insert("duplicate_count".into(), serde_json::json!(c.duplicate));
                obj.insert("failed_count".into(), serde_json::json!(c.failed));
                obj.insert("total_count".into(), serde_json::json!(c.total));
            }
            bus.emit("job:completed", payload);
        }
    }

    // ── Owner-plane scanned-PDF OCR projection ────────────────────────────
    //
    // Same shape as the knowledge-import projection above: per-page OCR
    // state has its own durable fact layer (`knowledge_source_ocr_pages` in
    // sessions.db). This row is only the unified background-job lifecycle
    // projection, covering both the initial import round and later
    // per-page retries.

    pub fn spawn_knowledge_ocr(kb_id: &str, source_id: &str, page_count: u32) -> Option<String> {
        let db = super::get_async_jobs_db()?;
        let job_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();
        let job = BackgroundJob {
            job_id: job_id.clone(),
            kind: JobKind::Tool,
            subagent_run_id: None,
            group_id: None,
            session_id: None,
            agent_id: None,
            tool_name: "knowledge_source_ocr".to_string(),
            tool_call_id: None,
            args_json: serde_json::json!({
                "kbId": kb_id,
                "sourceId": source_id,
                "pageCount": page_count,
            })
            .to_string(),
            status: JobStatus::Running,
            result_preview: None,
            result_path: None,
            error: None,
            created_at: now,
            completed_at: None,
            injected: true,
            origin: "owner".to_string(),
            approval_origin: None,
            incognito: false,
            pid: None,
            cancel_requested: false,
        };
        if let Err(e) = db.insert(&job) {
            crate::app_warn!(
                "async_jobs",
                "knowledge_ocr",
                "Failed to insert knowledge OCR job for source {}: {}",
                source_id,
                e
            );
            return None;
        }
        super::events::emit_created(
            &job_id,
            JobKind::Tool,
            "knowledge_source_ocr",
            JobStatus::Running.as_str(),
            None,
        );
        Some(job_id)
    }

    pub fn finish_knowledge_ocr(
        job_id: &str,
        status: JobStatus,
        result_preview: Option<&str>,
        error: Option<&str>,
    ) {
        let Some(db) = super::get_async_jobs_db() else {
            return;
        };
        let now = chrono::Utc::now().timestamp();
        if let Err(e) = db.update_terminal(job_id, status, result_preview, None, error, now) {
            crate::app_warn!(
                "async_jobs",
                "knowledge_ocr",
                "Failed to finish knowledge OCR job {}: {}",
                job_id,
                e
            );
            return;
        }
        let _ = db.mark_injected(job_id);
        super::events::emit_completed(
            job_id,
            JobKind::Tool,
            "knowledge_source_ocr",
            status.as_str(),
            None,
        );
    }

    // ── Subagent projection (R6) ───────────────────────────────────────────
    //
    // A background subagent run gets a one-way scheduling projection here so it
    // shows up in the unified job surface (`list` / `status` / `cancel`) and the
    // future R4 panel. `subagent_runs` stays the execution truth source; this
    // projection carries status / lifecycle ONLY — never run content.

    /// Map a subagent run's status onto the unified job status for its
    /// projection. `Queued` → `Queued` (R7.2, parked for a slot); `Spawning`/
    /// `Running` → `Running` (active); terminal states map 1:1 (`Error`→`Failed`,
    /// `Timeout`→`TimedOut`, `Killed`→`Cancelled`).
    fn subagent_status_as_job(status: SubagentStatus) -> JobStatus {
        match status {
            SubagentStatus::Queued => JobStatus::Queued,
            SubagentStatus::Spawning | SubagentStatus::Running => JobStatus::Running,
            SubagentStatus::Completed => JobStatus::Completed,
            SubagentStatus::Error => JobStatus::Failed,
            SubagentStatus::Timeout => JobStatus::TimedOut,
            SubagentStatus::Killed => JobStatus::Cancelled,
        }
    }

    /// Create a one-way scheduling projection for a background subagent run (R6).
    ///
    /// The caller gates this (only user-delegated, non-incognito runs are
    /// projected — `!skip_parent_injection && !parent_incognito`). The row holds
    /// NO run content: `args_json` is empty and result/error are never set (they
    /// live only in `subagent_runs`). `injected=true` keeps it out of the
    /// tool-job injection/replay path entirely — the subagent does its own
    /// `inject_and_run_parent`. No-op if the jobs DB is uninitialized.
    ///
    /// `group_id` (R5) links this child to its `Group` join coordinator when the
    /// run is part of a `batch_spawn` fan-out (`None` for a standalone spawn).
    pub fn project_subagent_spawn(
        run_id: &str,
        parent_session_id: &str,
        parent_agent_id: &str,
        child_agent_id: &str,
        status: SubagentStatus,
        group_id: Option<&str>,
    ) -> Result<()> {
        let Some(db) = super::get_async_jobs_db() else {
            return Ok(());
        };
        let job = BackgroundJob {
            job_id: super::spawn::new_job_id(),
            kind: JobKind::Subagent,
            subagent_run_id: Some(run_id.to_string()),
            group_id: group_id.map(|g| g.to_string()),
            session_id: Some(parent_session_id.to_string()),
            agent_id: Some(parent_agent_id.to_string()),
            // Label only — the task content is NOT copied (lives in subagent_runs).
            tool_name: format!("subagent:{child_agent_id}"),
            tool_call_id: None,
            args_json: "{}".to_string(),
            status: Self::subagent_status_as_job(status),
            result_preview: None,
            result_path: None,
            error: None,
            created_at: chrono::Utc::now().timestamp(),
            completed_at: None,
            injected: true,
            origin: JobOrigin::Explicit.as_str().to_string(),
            approval_origin: None,
            incognito: false,
            pid: None,
            cancel_requested: false,
        };
        db.insert(&job)
    }

    /// One-way sync (R6): propagate a subagent run's status onto its projection,
    /// keyed by `subagent_run_id`. Best-effort and a no-op when no projection
    /// exists (foreground / internal / incognito runs are never projected).
    /// NEVER writes run content back — status + completed_at only.
    pub fn sync_subagent_projection(run_id: &str, status: SubagentStatus) {
        let Some(db) = super::get_async_jobs_db() else {
            return;
        };
        let job_status = Self::subagent_status_as_job(status);
        let completed_at = job_status
            .is_terminal()
            .then(|| chrono::Utc::now().timestamp());
        if let Err(e) = db.update_subagent_projection_status(run_id, job_status, completed_at) {
            crate::app_warn!(
                "async_jobs",
                "subagent_projection",
                "Failed to sync projection for subagent run {}: {}",
                run_id,
                e
            );
        }
        // R5: a grouped child reaching a terminal state may be the last one —
        // check whether its `Group` can now complete and fire ONE merged
        // injection. Idempotent: `try_complete_group` waits for the seal and the
        // single-winner CAS guards against double-fire across concurrent
        // siblings.
        if job_status.is_terminal() {
            match db.group_id_for_subagent_run(run_id) {
                Ok(Some(group_id)) => Self::try_complete_group(&group_id),
                Ok(None) => {}
                Err(e) => crate::app_warn!(
                    "async_jobs",
                    "group",
                    "Failed to look up group for subagent run {}: {}",
                    run_id,
                    e
                ),
            }
        }
    }

    /// R8 follow-up: reflect a *background subagent's* INNER tool approval on its
    /// Background Job projection. A background subagent runs its own turns in a
    /// child session, so its inner approvals don't pass through the job-thread's
    /// thread-local approval bridge (that only covers `kind=Tool` jobs run by
    /// [`super::spawn::run_job_to_completion`]); instead an EventBus watcher calls
    /// this on `approval_required` (`parked=true`: running → awaiting_approval)
    /// and `approval:resolved` (`parked=false`: awaiting_approval → running).
    ///
    /// **Pure projection — never gates execution.** The inner approval still
    /// block-and-waits in the child session exactly as before; this only moves
    /// the projection's *label* so the panel / `job_status` show "等待审批"
    /// instead of "运行中", mirroring R8's background-`exec` behaviour. No-op
    /// unless `child_session_id` belongs to an active, *projected* subagent run
    /// (foreground / internal / incognito runs and every non-subagent approval —
    /// including R8's background `exec`, whose approval carries its *parent*
    /// session — fall straight through). The status flips reuse the kind-agnostic
    /// `mark_awaiting_approval` / `resume_from_awaiting_approval` WHERE-guards, so
    /// a run that already settled terminal (or a duplicate event) is a safe no-op.
    pub fn reflect_subagent_inner_approval(child_session_id: &str, parked: bool) {
        let Some(sdb) = crate::globals::get_session_db() else {
            return;
        };
        let run = match sdb.find_active_run_by_child_session(child_session_id) {
            Ok(Some(run)) => run,
            Ok(None) => return, // not a subagent child session, or run already settled
            Err(e) => {
                crate::app_warn!(
                    "async_jobs",
                    "subagent_projection",
                    "Failed to resolve subagent run for child session {}: {}",
                    child_session_id,
                    e
                );
                return;
            }
        };
        let Some(db) = super::get_async_jobs_db() else {
            return;
        };
        let job = match db.get_subagent_projection(&run.run_id) {
            Ok(Some(job)) => job,
            Ok(None) => return, // run isn't projected (internal / incognito) → no-op
            Err(e) => {
                crate::app_warn!(
                    "async_jobs",
                    "subagent_projection",
                    "Failed to load projection for subagent run {}: {}",
                    run.run_id,
                    e
                );
                return;
            }
        };
        let flipped = if parked {
            db.mark_awaiting_approval(&job.job_id)
        } else {
            db.resume_from_awaiting_approval(&job.job_id)
        };
        match flipped {
            // Only emit when the status actually changed. `false` ⇒ the row was
            // already in the target state or has settled terminal (frozen) — no
            // event, so the panel doesn't flicker awaiting↔running spuriously.
            Ok(true) => {
                let status = if parked {
                    JobStatus::AwaitingApproval
                } else {
                    JobStatus::Running
                };
                super::events::emit_updated(
                    &job.job_id,
                    JobKind::Subagent,
                    &job.tool_name,
                    status.as_str(),
                    job.session_id.as_deref(),
                );
            }
            Ok(false) => {}
            Err(e) => crate::app_warn!(
                "async_jobs",
                "subagent_projection",
                "Failed to flip projection {} (parked={}): {}",
                job.job_id,
                parked,
                e
            ),
        }
    }

    // ── Group fan-out (R5) ─────────────────────────────────────────────────
    //
    // A `batch_spawn` of N background subagents becomes a `Group`: one
    // coordinator row plus N `Subagent` child projections sharing its `job_id`
    // in `group_id`. When the group is sealed (all children spawned) and every
    // child is terminal, a single CAS-claimed winner fires ONE merged
    // `<task-notification>` summarizing every child (join-all-settle), instead
    // of N separate billed injection turns. The group row holds NO run content;
    // child results are read from `subagent_runs` (the truth source) only at
    // injection time.

    /// `child_agent_id` label carried into `inject_and_run_parent` for a Group's
    /// merged injection. Not `wakeup` and not the `tool_job:` prefix, so the
    /// frontend renders it through the standard `subagent_result` completion
    /// pill (the merged envelope is shaped like a single subagent result).
    const GROUP_CHILD_AGENT_ID: &'static str = "batch";

    /// Create a `Group` join coordinator (R5) for a `batch_spawn` fan-out and
    /// return its id. The group owns no work; its children are `Subagent`
    /// projections sharing this id in `group_id`. Status starts `Running` with
    /// `args_json = {"sealed":false}` until [`seal_group`] flips it (all children
    /// spawned). `injected=true` keeps the group out of the tool-job
    /// injection/replay path — the merged injection is fired directly by
    /// [`try_complete_group`]. Returns `None` (caller falls back to per-child
    /// injection) when the jobs DB is uninitialized or the insert fails.
    pub fn spawn_group(parent_session_id: &str, parent_agent_id: &str) -> Option<String> {
        let db = super::get_async_jobs_db()?;
        let group_id = super::spawn::new_job_id();
        let job = BackgroundJob {
            job_id: group_id.clone(),
            kind: JobKind::Group,
            subagent_run_id: None,
            group_id: None,
            session_id: Some(parent_session_id.to_string()),
            agent_id: Some(parent_agent_id.to_string()),
            tool_name: "subagent:batch".to_string(),
            tool_call_id: None,
            args_json: "{\"sealed\":false}".to_string(),
            status: JobStatus::Running,
            result_preview: None,
            result_path: None,
            error: None,
            created_at: chrono::Utc::now().timestamp(),
            completed_at: None,
            injected: true,
            origin: JobOrigin::Explicit.as_str().to_string(),
            approval_origin: None,
            incognito: false,
            pid: None,
            cancel_requested: false,
        };
        match db.insert(&job) {
            Ok(()) => {
                // R3: announce the new batch on the unified bus.
                super::events::emit_created(
                    &group_id,
                    JobKind::Group,
                    &job.tool_name,
                    JobStatus::Running.as_str(),
                    Some(parent_session_id),
                );
                Some(group_id)
            }
            Err(e) => {
                crate::app_warn!(
                    "async_jobs",
                    "group",
                    "Failed to create group row {}: {}",
                    group_id,
                    e
                );
                None
            }
        }
    }

    /// Seal a group (all children spawned) and run one completion check — covers
    /// the case where every child already settled before the spawn loop
    /// finished. Called once by `batch_spawn` after spawning all children.
    pub fn seal_group(group_id: &str) {
        if let Some(db) = super::get_async_jobs_db() {
            if let Err(e) = db.mark_group_sealed(group_id) {
                crate::app_warn!(
                    "async_jobs",
                    "group",
                    "Failed to seal group {}: {}",
                    group_id,
                    e
                );
                return;
            }
        }
        Self::try_complete_group(group_id);
    }

    /// R5: child progress for a `Group`, surfaced by `job_status`. Returns
    /// `(total, terminal, completed, failed)` over the group's child
    /// projections — `failed` counts every non-`Completed` terminal child
    /// (error / timeout / cancelled / interrupted). `None` if the jobs DB is
    /// uninitialized.
    pub fn group_progress(group_id: &str) -> Option<(usize, usize, usize, usize)> {
        let db = super::get_async_jobs_db()?;
        let children = db.group_children(group_id).ok()?;
        let total = children.len();
        let terminal = children.iter().filter(|c| c.status.is_terminal()).count();
        let completed = children
            .iter()
            .filter(|c| c.status == JobStatus::Completed)
            .count();
        Some((
            total,
            terminal,
            completed,
            terminal.saturating_sub(completed),
        ))
    }

    /// Join coordinator: if a sealed group's children are all terminal,
    /// atomically claim completion (single winner) and fire ONE merged
    /// injection. No-op until sealed, when the group is already terminal, or
    /// when the claim is lost (a sibling won, or a cancel marked it terminal
    /// first). Reads child results from `subagent_runs` only at build time so
    /// the projection stays content-free.
    fn try_complete_group(group_id: &str) {
        let Some(db) = super::get_async_jobs_db() else {
            return;
        };
        let Ok(Some(group)) = db.load(group_id) else {
            return;
        };
        if group.kind != JobKind::Group || group.status.is_terminal() {
            return;
        }
        // Not sealed → children still being spawned; `seal_group` will re-check.
        if !group_is_sealed(&group.args_json) {
            return;
        }
        let children = match db.group_children(group_id) {
            Ok(c) => c,
            Err(e) => {
                crate::app_warn!(
                    "async_jobs",
                    "group",
                    "Failed to load children for group {}: {}",
                    group_id,
                    e
                );
                return;
            }
        };
        // R3: emit N-of-M progress on the unified bus on every settle (reusing
        // the children load), so the R4 panel can render a live batch progress
        // bar. Cheap and idempotent — the panel just overwrites the last value.
        let total = children.len();
        let terminal = children.iter().filter(|c| c.status.is_terminal()).count();
        if total > 0 {
            super::events::emit_progress(
                group_id,
                JobKind::Group,
                group.session_id.as_deref(),
                terminal,
                total,
            );
        }
        if terminal != total {
            return;
        }
        // Claim before any delivery work — exactly one caller proceeds.
        match db.claim_group_completion(group_id, chrono::Utc::now().timestamp()) {
            Ok(true) => {}
            Ok(false) => return,
            Err(e) => {
                crate::app_warn!(
                    "async_jobs",
                    "group",
                    "Failed to claim completion for group {}: {}",
                    group_id,
                    e
                );
                return;
            }
        }
        // Wake any `job_status(action='wait')` parked on the group id.
        super::wait::notify_completion(group_id);
        // R3: the batch finished — announce terminal on the unified bus.
        super::events::emit_completed(
            group_id,
            JobKind::Group,
            &group.tool_name,
            JobStatus::Completed.as_str(),
            group.session_id.as_deref(),
        );

        // Empty group (every child failed to project) → nothing to inject.
        if children.is_empty() {
            return;
        }

        let run_ids: Vec<String> = children
            .iter()
            .filter_map(|c| c.subagent_run_id.clone())
            .collect();
        // If the parent already collected every child result (wait_all / check /
        // result), the merged injection is redundant — drain the fetched marks
        // (the suppressed per-child injections would otherwise leak them) and
        // skip the billed turn. A partial collection still injects the full
        // summary. The `fetched == len` test is sound because `run_ids` are
        // distinct: each child is a separate run (unique `Uuid` per spawn) with
        // its own projection row, so `take_runs_fetched`'s distinct-removal count
        // can equal the length only when every id was present.
        let fetched = crate::subagent::take_runs_fetched(&run_ids);
        if !run_ids.is_empty() && fetched == run_ids.len() {
            crate::app_info!(
                "async_jobs",
                "group",
                "Group {} fully fetched by parent; skipping merged injection",
                group_id
            );
            return;
        }

        let Some(session_db) = crate::get_session_db() else {
            return;
        };
        let (Some(parent_session_id), Some(parent_agent_id)) =
            (group.session_id.clone(), group.agent_id.clone())
        else {
            return;
        };
        let push_message = Self::build_group_push_message(group_id, &children, session_db);

        // Fire on a dedicated OS thread + current-thread runtime, mirroring the
        // subagent injection path (the future isn't `Send`: inject → chat →
        // spawn). Fire-and-forget: the group row is already `Completed` +
        // `injected=true` (out of replay), and `inject_and_run_parent` dedups
        // re-queued attempts by `run_id` (the group id).
        let session_db = session_db.clone();
        let group_id_owned = group_id.to_string();
        std::thread::spawn(move || {
            match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => {
                    let _ = rt.block_on(crate::subagent::injection::inject_and_run_parent(
                        parent_session_id,
                        parent_agent_id,
                        Self::GROUP_CHILD_AGENT_ID.to_string(),
                        group_id_owned,
                        push_message,
                        session_db,
                        None,
                    ));
                }
                Err(e) => crate::app_error!(
                    "async_jobs",
                    "group",
                    "Failed to build runtime for group injection: {}",
                    e
                ),
            }
        });
    }

    /// Build the merged `<task-notification>` for a completed group: a single
    /// `<subagent-result>`-shaped envelope (so the existing frontend pill
    /// renders it) whose `<result>` body enumerates every child's terminal
    /// status + result/error. Join-all-settle: failures are included alongside
    /// successes, never dropped. Child content is read from `subagent_runs`.
    fn build_group_push_message(
        group_id: &str,
        children: &[BackgroundJob],
        session_db: &crate::session::SessionDB,
    ) -> String {
        use crate::subagent::SubagentStatus;
        let run_ids: Vec<String> = children
            .iter()
            .filter_map(|c| c.subagent_run_id.clone())
            .collect();
        let runs = session_db
            .get_subagent_runs_batch(&run_ids)
            .unwrap_or_default();

        let total = children.len();
        let mut completed = 0usize;
        let mut failed = 0usize;
        let mut body = String::new();
        for (i, child) in children.iter().enumerate() {
            let idx = i + 1;
            let run = child
                .subagent_run_id
                .as_deref()
                .and_then(|rid| runs.get(rid));
            match run {
                Some(r) => {
                    if matches!(r.status, SubagentStatus::Completed) {
                        completed += 1;
                    } else {
                        failed += 1;
                    }
                    let dur = format!("{:.1}s", r.duration_ms.unwrap_or(0) as f64 / 1000.0);
                    // Include the task (truncated) + label so the model can map a
                    // numbered child back to what it ran — load-bearing for
                    // heterogeneous batches where "[2] coder — error" alone is
                    // not actionable ("handle failures as you see fit").
                    let label = r
                        .label
                        .as_deref()
                        .map(|l| format!(" [{}]", escape_xml(l)))
                        .unwrap_or_default();
                    body.push_str(&format!(
                        "[{}] {}{} — {} ({}) — task: {}\n",
                        idx,
                        escape_xml(&r.child_agent_id),
                        label,
                        escape_xml(r.status.as_str()),
                        escape_xml(&dur),
                        escape_xml(&truncate_chars(&r.task, 120)),
                    ));
                    if let Some(res) = r.result.as_deref().map(str::trim).filter(|s| !s.is_empty())
                    {
                        body.push_str(&escape_xml(res));
                        body.push('\n');
                    }
                    if let Some(err) = r.error.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                        body.push_str(&escape_xml(err));
                        body.push('\n');
                    }
                    body.push('\n');
                }
                None => {
                    failed += 1;
                    body.push_str(&format!("[{}] (sub-agent run record missing)\n\n", idx));
                }
            }
        }

        let overall_status = if failed == 0 { "completed" } else { "error" };
        let summary = format!(
            "{} background sub-agents finished ({} completed, {} failed). Review each \
             result below and handle any failures as you see fit.",
            total, completed, failed
        );
        format!(
            "<subagent-result>\n\
             <run-id>{}</run-id>\n\
             <agent>batch</agent>\n\
             <status>{}</status>\n\
             <task>Batch of {} background sub-agents</task>\n\
             <summary>{}</summary>\n\
             <result>\n{}</result>\n\
             </subagent-result>",
            escape_xml(group_id),
            overall_status,
            total,
            escape_xml(&summary),
            body.trim_end()
        )
    }
}

/// Whether a group's `args_json` marks it sealed (all children spawned).
fn group_is_sealed(args_json: &str) -> bool {
    serde_json::from_str::<Value>(args_json)
        .ok()
        .and_then(|v| v.get("sealed").and_then(|s| s.as_bool()))
        .unwrap_or(false)
}

/// Minimal XML-text escaping matching `subagent::injection`'s encoder (and the
/// frontend's `decodeXmlishText` decoder): `&` first, then `<` / `>`. Keeps the
/// merged envelope parseable — escaped child content can't contain a literal
/// `</result>` that would truncate the body.
fn escape_xml(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Truncate to at most `max` chars (char-based, never splits a UTF-8 boundary),
/// appending `…` when cut. Used to keep a batch child's task line compact.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subagent::{SubagentRun, SubagentStatus};

    #[test]
    fn group_is_sealed_reads_the_flag() {
        assert!(group_is_sealed("{\"sealed\":true}"));
        assert!(!group_is_sealed("{\"sealed\":false}"));
        // Missing / malformed → not sealed (fail-safe: a group never auto-
        // completes before `seal_group` ran).
        assert!(!group_is_sealed("{}"));
        assert!(!group_is_sealed("not json"));
    }

    #[test]
    fn escape_xml_escapes_amp_first() {
        assert_eq!(escape_xml("a & b < c > d"), "a &amp; b &lt; c &gt; d");
        // `&` must be escaped before `<`/`>` so `&lt;` isn't double-encoded.
        assert_eq!(escape_xml("<"), "&lt;");
    }

    #[test]
    fn truncate_chars_is_utf8_safe() {
        assert_eq!(truncate_chars("hello", 10), "hello");
        assert_eq!(truncate_chars("hello", 5), "hello");
        assert_eq!(truncate_chars("hello world", 5), "hell…");
        // Multibyte chars must not be split mid-codepoint.
        assert_eq!(truncate_chars("日本語テスト", 3), "日本…");
    }

    fn run(
        id: &str,
        agent: &str,
        status: SubagentStatus,
        result: Option<&str>,
        error: Option<&str>,
        dur: u64,
    ) -> SubagentRun {
        SubagentRun {
            run_id: id.into(),
            parent_session_id: "s".into(),
            parent_agent_id: "ha-main".into(),
            child_agent_id: agent.into(),
            child_session_id: format!("{id}-child"),
            task: "do work".into(),
            status,
            result: result.map(Into::into),
            error: error.map(Into::into),
            depth: 1,
            model_used: None,
            started_at: "2026-01-01T00:00:00Z".into(),
            finished_at: None,
            duration_ms: Some(dur),
            label: None,
            attachment_count: 0,
            input_tokens: None,
            output_tokens: None,
        }
    }

    fn child(job_id: &str, run_id: &str, group_id: &str) -> BackgroundJob {
        BackgroundJob {
            job_id: job_id.into(),
            kind: JobKind::Subagent,
            subagent_run_id: Some(run_id.into()),
            group_id: Some(group_id.into()),
            session_id: Some("s".into()),
            agent_id: Some("ha-main".into()),
            tool_name: "subagent:x".into(),
            tool_call_id: None,
            args_json: "{}".into(),
            status: JobStatus::Completed,
            result_preview: None,
            result_path: None,
            error: None,
            created_at: 0,
            completed_at: Some(1),
            injected: true,
            origin: JobOrigin::Explicit.as_str().to_string(),
            approval_origin: None,
            incognito: false,
            pid: None,
            cancel_requested: false,
        }
    }

    /// The merged message is join-all-settle: a single frontend-parseable
    /// `<subagent-result>` envelope whose `<result>` body enumerates EVERY
    /// child (successes + failures), overall status flips to `error` when any
    /// child failed, and child content is XML-escaped so it can't truncate the
    /// body.
    #[test]
    fn build_group_push_message_includes_every_child_and_escapes() {
        let dir = tempfile::tempdir().unwrap();
        let sdb =
            crate::session::SessionDB::open_ephemeral_for_test(&dir.path().join("s.db")).unwrap();
        let mut r1 = run(
            "r1",
            "researcher",
            SubagentStatus::Completed,
            Some("found 3 papers"),
            None,
            1200,
        );
        r1.task = "survey recent papers".into();
        r1.label = Some("research-step".into());
        sdb.insert_subagent_run(&r1).unwrap();
        sdb.insert_subagent_run(&run(
            "r2",
            "coder",
            SubagentStatus::Error,
            None,
            Some("compile <failed> & bailed"),
            800,
        ))
        .unwrap();
        let children = vec![child("c1", "r1", "g"), child("c2", "r2", "g")];

        let msg = JobManager::build_group_push_message("g", &children, &sdb);

        // Frontend-parseable envelope (renders via the subagent_result pill).
        assert!(msg.starts_with("<subagent-result>"));
        assert!(msg.contains("</subagent-result>"));
        // Overall status = error because one child failed → red pill.
        assert!(msg.contains("<status>error</status>"));
        // Join-all-settle summary counts both outcomes.
        assert!(msg.contains("2 background sub-agents finished (1 completed, 1 failed)"));
        // Both children present; the FAILURE is not dropped.
        assert!(msg.contains("researcher"));
        assert!(msg.contains("coder"));
        assert!(msg.contains("found 3 papers"));
        // Each child carries its task + label so the model can map idx → work.
        assert!(msg.contains("survey recent papers"));
        assert!(msg.contains("research-step"));
        assert!(msg.contains("task:"));
        // Child content is XML-escaped (no literal `<`/`>`/`&`).
        assert!(msg.contains("compile &lt;failed&gt; &amp; bailed"));
        // The outer <result> wraps the whole body (no inner </result> leaked).
        let first_result_close = msg.find("</result>").unwrap();
        let envelope_close = msg.find("</subagent-result>").unwrap();
        assert!(first_result_close < envelope_close);
        assert_eq!(msg.matches("</result>").count(), 1);
    }

    #[test]
    fn build_group_push_message_all_completed_is_status_completed() {
        let dir = tempfile::tempdir().unwrap();
        let sdb =
            crate::session::SessionDB::open_ephemeral_for_test(&dir.path().join("s.db")).unwrap();
        sdb.insert_subagent_run(&run(
            "r1",
            "w",
            SubagentStatus::Completed,
            Some("done"),
            None,
            10,
        ))
        .unwrap();
        let children = vec![child("c1", "r1", "g")];
        let msg = JobManager::build_group_push_message("g", &children, &sdb);
        assert!(msg.contains("<status>completed</status>"));
        assert!(msg.contains("1 background sub-agents finished (1 completed, 0 failed)"));
    }

    /// A child whose subagent_run record is missing is still counted (as a
    /// failure) and surfaced — never silently dropped.
    #[test]
    fn build_group_push_message_missing_run_record_is_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let sdb =
            crate::session::SessionDB::open_ephemeral_for_test(&dir.path().join("s.db")).unwrap();
        let children = vec![child("c1", "missing_run", "g")];
        let msg = JobManager::build_group_push_message("g", &children, &sdb);
        assert!(msg.contains("<status>error</status>"));
        assert!(msg.contains("(0 completed, 1 failed)"));
        assert!(msg.contains("run record missing"));
    }
}
