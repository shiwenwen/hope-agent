use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use cron::Schedule as CronExpression;
use std::str::FromStr;

use super::types::CronSchedule;

// ── Timestamp Parsing ──────────────────────────────────────────

/// Parse a timestamp string with flexible timezone offset formats.
/// Supports RFC 3339 (`+08:00`) and compact offset (`+0800`).
pub fn parse_flexible_timestamp(s: &str) -> Option<DateTime<Utc>> {
    // Try RFC 3339 first
    if let Ok(ts) = DateTime::parse_from_rfc3339(s) {
        return Some(ts.with_timezone(&Utc));
    }
    // Try normalizing compact offset like +0800 → +08:00
    let normalized = normalize_tz_offset(s);
    if normalized != s {
        if let Ok(ts) = DateTime::parse_from_rfc3339(&normalized) {
            return Some(ts.with_timezone(&Utc));
        }
    }
    None
}

/// Normalize compact timezone offsets: `+0800` → `+08:00`, `-0530` → `-05:30`
fn normalize_tz_offset(s: &str) -> String {
    let bytes = s.as_bytes();
    let len = bytes.len();
    // Match pattern: ...+HHMM or ...-HHMM at the end (4 digits after +/-)
    if len >= 5 {
        let sign_pos = len - 5;
        if (bytes[sign_pos] == b'+' || bytes[sign_pos] == b'-')
            && bytes[sign_pos + 1..].iter().all(|b| b.is_ascii_digit())
        {
            let mut result = String::from(&s[..sign_pos + 3]);
            result.push(':');
            result.push_str(&s[sign_pos + 3..]);
            return result;
        }
    }
    s.to_string()
}

// ── Schedule Computation ────────────────────────────────────────

/// Compute the next run time for a schedule, from a given reference time.
pub fn compute_next_run(schedule: &CronSchedule, after: &DateTime<Utc>) -> Option<DateTime<Utc>> {
    match schedule {
        CronSchedule::At { timestamp } => {
            let ts = parse_flexible_timestamp(timestamp)?;
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
