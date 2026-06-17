//! Typed terminal error for async tool jobs.
//!
//! Replaces the fragile `e.contains("was cancelled")` /
//! `e.contains("exceeded max_job_secs")` string-matching that
//! `finalize_job` used to re-derive the terminal status (MISC-7). The
//! `run_job_to_completion` / auto-background workers now construct the right
//! variant directly from the `select!` arm that fired, so the terminal
//! [`JobStatus`] comes from a type rather than re-parsing a message.

use super::types::JobStatus;
use crate::tools::rejection::ToolRejection;

/// Why an async tool job ended without a successful result.
#[derive(Debug)]
pub enum JobError {
    /// The job's cancellation token tripped (turn cancel, session delete,
    /// explicit `cancel_job`, …) and the dispatch future was dropped.
    Cancelled,
    /// The job exceeded its `max_job_secs` runtime budget.
    TimedOut { max_secs: u64 },
    /// The inner tool dispatch returned a typed [`ToolRejection`] (deny /
    /// policy / approval-timeout / mid-flight cancel). Carried verbatim so the
    /// "STOP and wait" semantics survive into the injected
    /// `<task-notification>` (ASYNC-4). Maps to `Failed` status — there is no
    /// separate `Denied` terminal state, avoiding an exhaustive enum bump
    /// across the status match sites.
    DeniedByUser { rejection: ToolRejection },
    /// Any other dispatch failure (tool error, runtime issue, …).
    Failed { message: String },
}

impl JobError {
    /// Terminal [`JobStatus`] this error maps to. `DeniedByUser` folds
    /// into `Failed` (see the variant doc).
    pub fn to_status(&self) -> JobStatus {
        match self {
            JobError::Cancelled => JobStatus::Cancelled,
            JobError::TimedOut { .. } => JobStatus::TimedOut,
            JobError::DeniedByUser { .. } | JobError::Failed { .. } => JobStatus::Failed,
        }
    }

    /// Text stored in the job's `error` column and surfaced in the
    /// `<task-notification>` injection. `DeniedByUser` routes through
    /// [`ToolRejection::to_tool_result`] so the "STOP and wait" guidance rides
    /// into the conversation; the others keep short human-readable messages
    /// that `build_tool_job_push_message`'s status arms wrap.
    pub fn display_for_injection(&self) -> String {
        match self {
            JobError::Cancelled => "Job was cancelled.".to_string(),
            JobError::TimedOut { max_secs } => format!("exceeded max_job_secs ({}s)", max_secs),
            JobError::DeniedByUser { rejection } => rejection.to_tool_result(),
            JobError::Failed { message } => message.clone(),
        }
    }

    /// Classify the inner dispatch error. A typed [`ToolRejection`] is
    /// preserved as `DeniedByUser` so its STOP semantics survive into the
    /// injection; any other `anyhow::Error` becomes a generic `Failed`.
    pub fn from_dispatch_error(e: anyhow::Error) -> Self {
        match e.downcast::<ToolRejection>() {
            Ok(rejection) => JobError::DeniedByUser { rejection },
            Err(e) => JobError::Failed {
                message: e.to_string(),
            },
        }
    }

    /// Convert back into an `anyhow::Error` for the auto-background *inline*
    /// return path: when the worker finishes within the foreground budget the
    /// result is handed straight back to the caller rather than injected, so
    /// the typed error has to collapse to `anyhow`. `DeniedByUser` restores the
    /// original [`ToolRejection`] so the model still gets STOP-and-wait; the
    /// others fall back to a plain message.
    pub fn into_inline_error(self) -> anyhow::Error {
        match self {
            JobError::DeniedByUser { rejection } => rejection.into(),
            JobError::Cancelled => anyhow::anyhow!("Async tool job was cancelled"),
            JobError::TimedOut { max_secs } => {
                anyhow::anyhow!("Async tool job exceeded max_job_secs ({}s)", max_secs)
            }
            JobError::Failed { message } => anyhow::anyhow!(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping_folds_denied_into_failed() {
        assert_eq!(JobError::Cancelled.to_status(), JobStatus::Cancelled);
        assert_eq!(
            JobError::TimedOut { max_secs: 30 }.to_status(),
            JobStatus::TimedOut
        );
        assert_eq!(
            JobError::Failed {
                message: "boom".into()
            }
            .to_status(),
            JobStatus::Failed
        );
        assert_eq!(
            JobError::DeniedByUser {
                rejection: ToolRejection::DeniedByUser {
                    name: "exec".into()
                }
            }
            .to_status(),
            JobStatus::Failed,
            "DeniedByUser must fold into Failed — no separate Denied terminal state"
        );
    }

    #[test]
    fn denied_injection_text_preserves_stop_semantics() {
        let je = JobError::DeniedByUser {
            rejection: ToolRejection::DeniedByUser {
                name: "exec".into(),
            },
        };
        let s = je.display_for_injection();
        assert!(s.starts_with("Tool error: "), "needs Tool error: prefix");
        assert!(s.contains("STOP what you are doing and wait"));
        assert!(s.contains("Tool 'exec' execution denied by user"));
    }

    #[test]
    fn timeout_injection_text_carries_seconds() {
        let s = JobError::TimedOut { max_secs: 120 }.display_for_injection();
        assert!(s.contains("exceeded max_job_secs (120s)"));
    }

    #[test]
    fn from_dispatch_error_recovers_typed_rejection() {
        let err = ToolRejection::denied_by_user("exec"); // anyhow::Error
        match JobError::from_dispatch_error(err) {
            JobError::DeniedByUser { rejection } => {
                assert!(matches!(rejection, ToolRejection::DeniedByUser { .. }));
            }
            other => panic!("expected DeniedByUser, got {other:?}"),
        }
    }

    #[test]
    fn from_dispatch_error_falls_back_to_failed_for_plain_anyhow() {
        let err = anyhow::anyhow!("disk full");
        match JobError::from_dispatch_error(err) {
            JobError::Failed { message } => assert_eq!(message, "disk full"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn inline_error_restores_rejection_stop_semantics() {
        let je = JobError::DeniedByUser {
            rejection: ToolRejection::DeniedByUser {
                name: "exec".into(),
            },
        };
        let e = je.into_inline_error();
        // Downcasts back to a ToolRejection so the streaming loop renders the
        // STOP template instead of a generic "Tool error: <msg>".
        assert!(e.downcast_ref::<ToolRejection>().is_some());
    }
}
