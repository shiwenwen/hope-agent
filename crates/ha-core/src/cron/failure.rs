//! Cron failure classification (§5).
//!
//! A cron run's final error (whatever `run_chat_engine` surfaced after exhausting
//! its own failover, or the executor's own timeout) is classified into a small,
//! stable set of buckets. The class is used for:
//! - a more descriptive run-log `status` (a timeout reads as `timeout`, not a
//!   generic `error`);
//! - the auto-disable notification reason, so the user can tell a misconfigured
//!   job apart from a provider outage when a job is disabled after N failures.
//!
//! Classification is **diagnostic only** — it deliberately does NOT change the
//! auto-disable policy (still `max_failures` consecutive failures). Mis-tagging a
//! transient outage as "configuration" must never cause a premature disable.

/// Coarse failure category for a cron run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CronFailureClass {
    /// The run exceeded its per-run wall-clock budget (`job_timeout_secs`).
    Timeout,
    /// A failure that retrying on schedule won't fix on its own — the job is
    /// misconfigured (no model / no agent / invalid setup). Surfaced so the
    /// disable notification can point the user at config rather than infra.
    Configuration,
    /// Anything else — assumed transient (network / provider / rate limit), the
    /// default so an unrecognized error is never mistaken for a config problem.
    Transient,
}

impl CronFailureClass {
    /// Classify from the final error text. Case-insensitive substring match; the
    /// default is [`Transient`](Self::Transient) so unknown errors stay retryable
    /// in spirit and never masquerade as a config problem.
    pub fn classify(error: &str) -> Self {
        let e = error.to_lowercase();
        if e.contains("timed out") || e.contains("timeout") {
            return Self::Timeout;
        }
        // Permanent setup problems that re-running won't resolve.
        if e.contains("no model configured")
            || e.contains("no models configured")
            || e.contains("no model")
            || e.contains("agent not found")
            || e.contains("no agent")
        {
            return Self::Configuration;
        }
        Self::Transient
    }

    /// The `cron_run_logs.status` value for this class. Only [`Timeout`](Self::Timeout)
    /// gets a distinct `timeout` status; the others keep the historical `error`.
    /// **Readers that bucket failures must treat `timeout` as a failure too** — the
    /// dashboard `failed_runs` aggregation (`dashboard/queries.rs`) counts failures
    /// as a denylist (`status NOT IN ('success','running','empty','cancelled')`, so
    /// `error`/`timeout`/`no_session` all count) rather than an allowlist; the
    /// calendar dot colors any genuinely-failed run-log status as an error. A new
    /// failure status tag is therefore auto-counted — do NOT "restore" an
    /// `IN ('error','timeout')` allowlist, which silently drops `no_session` and
    /// inflates the success rate (the bug that denylist replaced).
    pub fn run_log_status(&self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Configuration | Self::Transient => "error",
        }
    }

    /// Stable wire key carried to the frontend (which localizes it) and used in
    /// logs. Stays a `&'static str` so it never allocates.
    pub fn key(&self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Configuration => "configuration",
            Self::Transient => "transient",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CronFailureClass as C;

    #[test]
    fn timeout_wins() {
        assert_eq!(C::classify("Cron job timed out after 300s"), C::Timeout);
        assert_eq!(C::classify("request TIMEOUT"), C::Timeout);
    }

    #[test]
    fn configuration_errors() {
        assert_eq!(C::classify("No model configured"), C::Configuration);
        assert_eq!(
            C::classify("No models configured for this agent"),
            C::Configuration
        );
        assert_eq!(C::classify("Agent not found: ha-foo"), C::Configuration);
    }

    #[test]
    fn unknown_defaults_to_transient() {
        assert_eq!(C::classify("connection reset by peer"), C::Transient);
        assert_eq!(C::classify("HTTP 529 overloaded"), C::Transient);
        assert_eq!(C::classify(""), C::Transient);
    }

    #[test]
    fn run_log_status_only_distinguishes_timeout() {
        assert_eq!(C::Timeout.run_log_status(), "timeout");
        assert_eq!(C::Configuration.run_log_status(), "error");
        assert_eq!(C::Transient.run_log_status(), "error");
    }

    #[test]
    fn keys_are_stable() {
        assert_eq!(C::Timeout.key(), "timeout");
        assert_eq!(C::Configuration.key(), "configuration");
        assert_eq!(C::Transient.key(), "transient");
    }
}
