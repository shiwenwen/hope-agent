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

use anyhow::Result;
use serde_json::Value;

use super::types::{BackgroundJob, JobKind, JobOrigin, JobStatus};
use crate::subagent::SubagentStatus;
use crate::tools::ToolExecContext;

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

    /// All active (`queued`/`running`/`cancelling`/`awaiting_approval`) jobs
    /// owned by a session. Empty when the DB is not initialized.
    pub fn list_active_by_session(session_id: &str) -> Result<Vec<BackgroundJob>> {
        match super::get_async_jobs_db() {
            Some(db) => db.list_active_by_session(session_id),
            None => Ok(Vec::new()),
        }
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

    // ── Subagent projection (R6) ───────────────────────────────────────────
    //
    // A background subagent run gets a one-way scheduling projection here so it
    // shows up in the unified job surface (`list` / `status` / `cancel`) and the
    // future R4 panel. `subagent_runs` stays the execution truth source; this
    // projection carries status / lifecycle ONLY — never run content.

    /// Map a subagent run's status onto the unified job status for its
    /// projection. `Spawning`/`Running` → `Running` (active); terminal states map
    /// 1:1 (`Error`→`Failed`, `Timeout`→`TimedOut`, `Killed`→`Cancelled`).
    fn subagent_status_as_job(status: SubagentStatus) -> JobStatus {
        match status {
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
            Ok(()) => Some(group_id),
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
        if !children.iter().all(|c| c.status.is_terminal()) {
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
        // summary.
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
                    body.push_str(&format!(
                        "[{}] {} — {} ({})\n",
                        idx,
                        escape_xml(&r.child_agent_id),
                        escape_xml(r.status.as_str()),
                        escape_xml(&dur),
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
        let sdb = crate::session::SessionDB::open(&dir.path().join("s.db")).unwrap();
        sdb.insert_subagent_run(&run(
            "r1",
            "researcher",
            SubagentStatus::Completed,
            Some("found 3 papers"),
            None,
            1200,
        ))
        .unwrap();
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
        let sdb = crate::session::SessionDB::open(&dir.path().join("s.db")).unwrap();
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
        let sdb = crate::session::SessionDB::open(&dir.path().join("s.db")).unwrap();
        let children = vec![child("c1", "missing_run", "g")];
        let msg = JobManager::build_group_push_message("g", &children, &sdb);
        assert!(msg.contains("<status>error</status>"));
        assert!(msg.contains("(0 completed, 1 failed)"));
        assert!(msg.contains("run record missing"));
    }
}
