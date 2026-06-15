use serde::{Deserialize, Serialize};

/// What kind of work a background job runs (R1 unified model). `Tool` is the
/// only kind wired for execution in this slice; `Subagent` (R6) and `Group`
/// (R5 fan-out + join) are the typed seams later slices fill — the `kind`
/// column lets one `background_jobs` table + one `JobManager` front them all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// A single backgrounded tool call (`exec` / `web_search` / …).
    #[default]
    Tool,
    /// A whole backgrounded subagent run, projected from `subagent_runs` (R6).
    Subagent,
    /// A fan-out of child jobs joined as one unit (R5).
    Group,
}

impl JobKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Subagent => "subagent",
            Self::Group => "group",
        }
    }

    /// Parse a stored `kind` string. Unknown / legacy values fall back to
    /// `Tool` (the only kind written before R1) so a stale row never breaks
    /// a load — mirrors the `JobStatus::parse` fallback discipline.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "tool" => Some(Self::Tool),
            "subagent" => Some(Self::Subagent),
            "group" => Some(Self::Group),
            _ => None,
        }
    }
}

/// Lifecycle status of a background job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// Reserved (row written) but waiting for a concurrency slot — the cap was
    /// full at spawn, so the job sits in the in-memory scheduler queue until a
    /// slot frees (per-session round-robin). NOT terminal. The queue holds the
    /// live `ToolExecContext` in memory, so a `Queued` row cannot survive a
    /// restart and is recovered as `Interrupted` like `Running`.
    Queued,
    Running,
    /// Cancellation has been requested and the running future has been
    /// signalled, but the runner has not yet finalized the row.
    Cancelling,
    /// Registered (row written) but execution is blocked waiting for a human
    /// approval decision. NOT terminal and not yet consuming wall-clock budget
    /// — distinguishes "running" from "waiting on a human" for
    /// job_status / dashboard / replay (a backgrounded exec parked on its
    /// command-level approval gate sits here, not in `Running`).
    AwaitingApproval,
    Completed,
    Failed,
    /// Job was running when the application restarted; the process state
    /// is unrecoverable.
    Interrupted,
    /// Job exceeded its configured wall-clock budget and was forcibly cancelled.
    TimedOut,
    /// Job was cancelled by the user/model before it completed.
    Cancelled,
}

impl JobStatus {
    /// SQL fragment enumerating all terminal status strings for a
    /// `status IN (...)` clause. Single source of truth so adding a new
    /// variant can't silently skip purge / replay logic.
    pub const TERMINAL_STATUS_SQL_LIST: &'static str =
        "'completed','failed','interrupted','timed_out','cancelled'";

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "cancelling" => Some(Self::Cancelling),
            "awaiting_approval" => Some(Self::AwaitingApproval),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "interrupted" => Some(Self::Interrupted),
            "timed_out" => Some(Self::TimedOut),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(
            self,
            Self::Queued | Self::Running | Self::Cancelling | Self::AwaitingApproval
        )
    }

    /// `(is_error, is_interrupt)` for the terminal PostToolUse hook fire (H4/H6).
    /// `Completed` → success; `Cancelled` / `Interrupted` → interrupted failure;
    /// everything else (`Failed` / `TimedOut`, plus non-terminal states that
    /// should never reach the fire site) → a plain (non-interrupt) failure.
    pub fn terminal_hook_flags(self) -> (bool, bool) {
        match self {
            Self::Completed => (false, false),
            Self::Cancelled | Self::Interrupted => (true, true),
            _ => (true, false),
        }
    }
}

/// A single background job row (R1 unified model — was `AsyncJob`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundJob {
    pub job_id: String,
    /// What kind of work this job runs. `Tool` for every row today; the column
    /// is the seam `Subagent` (R6) / `Group` (R5) fill later.
    pub kind: JobKind,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub tool_name: String,
    pub tool_call_id: Option<String>,
    pub args_json: String,
    pub status: JobStatus,
    /// Inline result preview (head + tail, capped at `inline_result_bytes`).
    pub result_preview: Option<String>,
    /// Path to the spooled full result on disk (when result exceeds inline cap).
    pub result_path: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub injected: bool,
    /// `auto_backgrounded` for sync calls that exceeded the budget,
    /// `explicit` for `run_in_background: true`,
    /// `policy_forced` for agent `always-background`.
    pub origin: String,
    /// How this job's execution was authorized (snake_case: `user` /
    /// `timeout_proceed` / `yolo` / `auto_approve` / `external_pre_approved`).
    /// `None` for jobs that never hit an approval gate. Column lands in A-7;
    /// real values written by B4 / F6 (TIMEOUT-2 audit).
    pub approval_origin: Option<String>,
    /// Whether the owning session is incognito — incognito jobs skip on-disk
    /// args/output persistence. Column lands in A-7; set by E4.
    pub incognito: bool,
    /// OS process id of the spawned child (exec), for restart orphan cleanup.
    /// Column lands in A-7; set by I3.
    pub pid: Option<i64>,
    /// Cross-process cancel flag — set via DB so another process's runner can
    /// observe cancellation. Column lands in A-7; set by I4.
    pub cancel_requested: bool,
}

/// Reason a job was created — primarily for telemetry / injection wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobOrigin {
    Explicit,
    PolicyForced,
    AutoBackgrounded,
}

impl JobOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::PolicyForced => "policy_forced",
            Self::AutoBackgrounded => "auto_backgrounded",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn async_job_status_as_str_parse_roundtrip() {
        for s in [
            JobStatus::Queued,
            JobStatus::Running,
            JobStatus::Cancelling,
            JobStatus::AwaitingApproval,
            JobStatus::Completed,
            JobStatus::Failed,
            JobStatus::Interrupted,
            JobStatus::TimedOut,
            JobStatus::Cancelled,
        ] {
            assert_eq!(JobStatus::parse(s.as_str()), Some(s));
        }
    }

    #[test]
    fn async_job_status_parse_unknown_returns_none() {
        assert!(JobStatus::parse("not-a-status").is_none());
        assert!(JobStatus::parse("").is_none());
    }

    #[test]
    fn terminal_hook_flags_map_status_to_error_and_interrupt() {
        // (is_error, is_interrupt)
        assert_eq!(JobStatus::Completed.terminal_hook_flags(), (false, false));
        assert_eq!(JobStatus::Failed.terminal_hook_flags(), (true, false));
        assert_eq!(JobStatus::TimedOut.terminal_hook_flags(), (true, false));
        assert_eq!(
            JobStatus::Cancelled.terminal_hook_flags(),
            (true, true),
            "cancellation is an interrupted failure"
        );
        assert_eq!(
            JobStatus::Interrupted.terminal_hook_flags(),
            (true, true),
            "restart interruption is an interrupted failure"
        );
    }

    #[test]
    fn is_terminal_marks_only_running_as_non_terminal() {
        assert!(!JobStatus::Queued.is_terminal());
        assert!(!JobStatus::Running.is_terminal());
        assert!(!JobStatus::Cancelling.is_terminal());
        assert!(!JobStatus::AwaitingApproval.is_terminal());
        for s in [
            JobStatus::Completed,
            JobStatus::Failed,
            JobStatus::Interrupted,
            JobStatus::TimedOut,
            JobStatus::Cancelled,
        ] {
            assert!(s.is_terminal(), "{:?} should be terminal", s);
        }
    }

    #[test]
    fn terminal_status_sql_list_covers_every_terminal_variant() {
        let list = JobStatus::TERMINAL_STATUS_SQL_LIST;
        for s in [
            JobStatus::Completed,
            JobStatus::Failed,
            JobStatus::Interrupted,
            JobStatus::TimedOut,
            JobStatus::Cancelled,
        ] {
            let fragment = format!("'{}'", s.as_str());
            assert!(
                list.contains(&fragment),
                "SQL list '{}' missing {}",
                list,
                fragment
            );
        }
        // Running must NOT be in the terminal list.
        assert!(!list.contains("'running'"));
        assert!(!list.contains("'cancelling'"));
    }

    #[test]
    fn job_origin_as_str_is_stable() {
        assert_eq!(JobOrigin::Explicit.as_str(), "explicit");
        assert_eq!(JobOrigin::PolicyForced.as_str(), "policy_forced");
        assert_eq!(JobOrigin::AutoBackgrounded.as_str(), "auto_backgrounded");
    }

    #[test]
    fn job_kind_as_str_parse_roundtrip() {
        for k in [JobKind::Tool, JobKind::Subagent, JobKind::Group] {
            assert_eq!(JobKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(JobKind::default(), JobKind::Tool);
        assert!(JobKind::parse("not-a-kind").is_none());
    }
}
