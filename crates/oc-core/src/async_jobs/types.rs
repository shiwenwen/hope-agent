use serde::{Deserialize, Serialize};

/// Lifecycle status of a backgrounded tool execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsyncJobStatus {
    Running,
    Completed,
    Failed,
    /// Job was running when the application restarted; the process state
    /// is unrecoverable.
    Interrupted,
    /// Job exceeded its configured wall-clock budget and was forcibly cancelled.
    TimedOut,
}

impl AsyncJobStatus {
    /// SQL fragment enumerating all terminal status strings for a
    /// `status IN (...)` clause. Single source of truth so adding a new
    /// variant can't silently skip purge / replay logic.
    pub const TERMINAL_STATUS_SQL_LIST: &'static str =
        "'completed','failed','interrupted','timed_out'";

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
            Self::TimedOut => "timed_out",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "interrupted" => Some(Self::Interrupted),
            "timed_out" => Some(Self::TimedOut),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

/// A single async tool job row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsyncJob {
    pub job_id: String,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub tool_name: String,
    pub tool_call_id: Option<String>,
    pub args_json: String,
    pub status: AsyncJobStatus,
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
            AsyncJobStatus::Running,
            AsyncJobStatus::Completed,
            AsyncJobStatus::Failed,
            AsyncJobStatus::Interrupted,
            AsyncJobStatus::TimedOut,
        ] {
            assert_eq!(AsyncJobStatus::parse(s.as_str()), Some(s));
        }
    }

    #[test]
    fn async_job_status_parse_unknown_returns_none() {
        assert!(AsyncJobStatus::parse("not-a-status").is_none());
        assert!(AsyncJobStatus::parse("").is_none());
    }

    #[test]
    fn is_terminal_marks_only_running_as_non_terminal() {
        assert!(!AsyncJobStatus::Running.is_terminal());
        for s in [
            AsyncJobStatus::Completed,
            AsyncJobStatus::Failed,
            AsyncJobStatus::Interrupted,
            AsyncJobStatus::TimedOut,
        ] {
            assert!(s.is_terminal(), "{:?} should be terminal", s);
        }
    }

    #[test]
    fn terminal_status_sql_list_covers_every_terminal_variant() {
        let list = AsyncJobStatus::TERMINAL_STATUS_SQL_LIST;
        for s in [
            AsyncJobStatus::Completed,
            AsyncJobStatus::Failed,
            AsyncJobStatus::Interrupted,
            AsyncJobStatus::TimedOut,
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
    }

    #[test]
    fn job_origin_as_str_is_stable() {
        assert_eq!(JobOrigin::Explicit.as_str(), "explicit");
        assert_eq!(JobOrigin::PolicyForced.as_str(), "policy_forced");
        assert_eq!(JobOrigin::AutoBackgrounded.as_str(), "auto_backgrounded");
    }
}
