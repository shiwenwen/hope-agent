use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use cron::Schedule as CronExpression;
use std::str::FromStr;

use super::types::CronSchedule;

// ── Schedule Computation ────────────────────────────────────────

/// Compute the next run time for a schedule, from a given reference time.
pub fn compute_next_run(schedule: &CronSchedule, after: &DateTime<Utc>) -> Option<DateTime<Utc>> {
    match schedule {
        CronSchedule::At { timestamp } => {
            let ts = DateTime::parse_from_rfc3339(timestamp)
                .ok()?
                .with_timezone(&Utc);
            if ts > *after { Some(ts) } else { None }
        }
        CronSchedule::Every { interval_ms } => {
            let dur = Duration::milliseconds(*interval_ms as i64);
            Some(*after + dur)
        }
        CronSchedule::Cron { expression, timezone } => {
            compute_next_cron(expression, timezone.as_deref(), after)
        }
    }
}

/// Parse cron expression and find the next occurrence after `after`.
fn compute_next_cron(
    expression: &str,
    _timezone: Option<&str>,
    after: &DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let schedule = CronExpression::from_str(expression).ok()?;
    // Find next occurrence after `after`
    schedule.after(after).next()
}

/// Validate a cron expression. Returns Ok if valid, Err with message if not.
pub fn validate_cron_expression(expression: &str) -> Result<()> {
    CronExpression::from_str(expression)
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("Invalid cron expression: {}", e))
}

/// Compute exponential backoff delay for failed jobs.
/// Returns milliseconds to add to next_run_at.
pub fn backoff_delay_ms(consecutive_failures: u32) -> u64 {
    let base_ms: u64 = 30_000; // 30 seconds
    let max_ms: u64 = 3_600_000; // 1 hour
    let delay = base_ms.saturating_mul(2u64.saturating_pow(consecutive_failures.min(20)));
    delay.min(max_ms)
}
