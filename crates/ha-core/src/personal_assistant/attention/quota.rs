//! One immutable attempt budget for every proactive interruption channel.
use crate::personal_assistant_im::{DeliveryDeferredReason, NotificationPolicy};
use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Timelike, Utc};
use chrono_tz::Tz;
use rusqlite::Connection;
const LIMIT: usize = 2;

fn attempts(conn: &Connection, now: i64) -> Result<Vec<i64>> {
    // No upper bound: a clock rollback cannot make future attempts free.
    let mut statement=conn.prepare("SELECT attempted_at FROM assistant_im_notification_attempts WHERE attempted_at>=?1 ORDER BY attempted_at")?;
    let rows = statement
        .query_map([now.saturating_sub(172_800)], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
fn instant(seconds: i64) -> Result<DateTime<Utc>> {
    DateTime::from_timestamp(seconds, 0).ok_or_else(|| anyhow!("Invalid notification clock"))
}
fn quiet(policy: &NotificationPolicy, hour: u32) -> bool {
    let (start, end) = (policy.quiet_start_hour, policy.quiet_end_hour);
    if start == end {
        true
    } else if start < end {
        hour >= start && hour < end
    } else {
        hour >= start || hour < end
    }
}
fn reason(
    policy: &NotificationPolicy,
    zone: Option<Tz>,
    timestamps: &[i64],
    now: i64,
) -> Result<Option<DeliveryDeferredReason>> {
    if policy.quiet_start_hour > 23 || policy.quiet_end_hour > 23 {
        bail!("Invalid quiet hours");
    }
    let Some(zone) = zone else {
        return Ok(Some(DeliveryDeferredReason::TimezoneRequired));
    };
    let local = instant(now)?.with_timezone(&zone);
    if quiet(policy, local.hour()) {
        return Ok(Some(DeliveryDeferredReason::QuietHours));
    }
    let mut same_day = 0;
    let mut rolling = 0;
    for timestamp in timestamps {
        if *timestamp >= now.saturating_sub(86_400) {
            rolling += 1;
        }
        if instant(*timestamp)?.with_timezone(&zone).date_naive() == local.date_naive() {
            same_day += 1;
        }
    }
    Ok((same_day >= LIMIT || rolling >= LIMIT).then_some(DeliveryDeferredReason::DailyLimit))
}
pub(crate) fn deferred_reason(
    conn: &Connection,
    policy: &NotificationPolicy,
    now: i64,
) -> Result<Option<DeliveryDeferredReason>> {
    reason(
        policy,
        policy
            .timezone
            .as_deref()
            .and_then(|name| name.parse().ok()),
        &attempts(conn, now)?,
        now,
    )
}
pub(crate) fn next_eligible_at(
    conn: &Connection,
    policy: &NotificationPolicy,
    now: i64,
) -> Result<Option<i64>> {
    let Some(zone) = policy
        .timezone
        .as_deref()
        .and_then(|name| name.parse::<Tz>().ok())
    else {
        return Ok(None);
    };
    if policy.quiet_start_hour == policy.quiet_end_hour {
        return Ok(None);
    }
    let timestamps = attempts(conn, now)?;
    let rolling: Vec<_> = timestamps
        .iter()
        .copied()
        .filter(|timestamp| *timestamp >= now.saturating_sub(86_400))
        .collect();
    let mut candidate = now;
    if rolling.len() >= LIMIT {
        candidate = candidate.max(
            rolling[rolling.len() - LIMIT]
                .checked_add(86_401)
                .ok_or_else(|| anyhow!("Notification time overflow"))?,
        );
    }
    let end = candidate.saturating_add(3 * 86_400);
    loop {
        if reason(policy, Some(zone), &timestamps, candidate)?.is_none() {
            return Ok(Some(candidate));
        }
        if candidate >= end {
            return Ok(None);
        }
        // Calendar/quiet boundaries occur on a local whole minute; scanning UTC
        // minutes naturally handles missing/repeated hours without guessing DST.
        candidate = candidate
            .div_euclid(60)
            .saturating_add(1)
            .saturating_mul(60);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn clock(value: &str) -> i64 {
        DateTime::parse_from_rfc3339(value).unwrap().timestamp()
    }
    fn fixture(zone: Option<&str>) -> (Connection, NotificationPolicy) {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE assistant_im_notification_attempts(delivery_id TEXT PRIMARY KEY,attempted_at INTEGER NOT NULL)").unwrap();
        (
            conn,
            NotificationPolicy {
                timezone: zone.map(str::to_owned),
                quiet_start_hour: 22,
                quiet_end_hour: 8,
                daily_limit: 2,
            },
        )
    }
    fn attempted(conn: &Connection, id: &str, at: i64) {
        conn.execute(
            "INSERT INTO assistant_im_notification_attempts VALUES(?1,?2)",
            rusqlite::params![id, at],
        )
        .unwrap();
    }
    #[test]
    fn changing_timezone_does_not_refund_the_rolling_window() {
        let (conn, mut policy) = fixture(Some("Asia/Shanghai"));
        attempted(&conn, "desktop", clock("2026-11-14T23:00:00Z"));
        attempted(&conn, "im-unknown", clock("2026-11-14T23:01:00Z"));
        let now = clock("2026-11-15T18:00:00Z");
        policy.timezone = Some("America/Los_Angeles".into());
        assert_eq!(
            deferred_reason(&conn, &policy, now).unwrap(),
            Some(DeliveryDeferredReason::DailyLimit)
        );
        assert_eq!(
            next_eligible_at(&conn, &policy, now).unwrap(),
            Some(clock("2026-11-15T23:00:01Z"))
        );
    }
    #[test]
    fn long_dst_day_keeps_the_local_date_cap_after_twenty_four_hours() {
        let (conn, mut policy) = fixture(Some("America/Los_Angeles"));
        policy.quiet_start_hour = 12;
        policy.quiet_end_hour = 13;
        attempted(&conn, "one", clock("2026-11-01T07:00:00Z"));
        attempted(&conn, "two", clock("2026-11-01T07:01:00Z"));
        let now = clock("2026-11-02T07:30:00Z");
        assert_eq!(
            deferred_reason(&conn, &policy, now).unwrap(),
            Some(DeliveryDeferredReason::DailyLimit)
        );
        assert_eq!(
            next_eligible_at(&conn, &policy, now).unwrap(),
            Some(clock("2026-11-02T08:00:00Z"))
        );
    }
    #[test]
    fn short_dst_day_does_not_refund_a_rolling_attempt_at_local_midnight() {
        let (conn, mut policy) = fixture(Some("America/Los_Angeles"));
        policy.quiet_start_hour = 12;
        policy.quiet_end_hour = 13;
        attempted(&conn, "one", clock("2026-03-08T08:00:00Z"));
        attempted(&conn, "two", clock("2026-03-08T08:01:00Z"));
        let now = clock("2026-03-09T07:00:00Z");
        assert_eq!(
            deferred_reason(&conn, &policy, now).unwrap(),
            Some(DeliveryDeferredReason::DailyLimit)
        );
        assert_eq!(
            next_eligible_at(&conn, &policy, now).unwrap(),
            Some(clock("2026-03-09T08:00:01Z"))
        );
    }
    #[test]
    fn future_unknown_attempts_remain_charged_after_clock_rollback() {
        let (conn, policy) = fixture(Some("Asia/Shanghai"));
        let now = clock("2026-11-15T03:00:00Z");
        attempted(&conn, "unknown-one", now + 3600);
        attempted(&conn, "unknown-two", now + 7200);
        assert_eq!(
            deferred_reason(&conn, &policy, now).unwrap(),
            Some(DeliveryDeferredReason::DailyLimit)
        );
        assert_eq!(
            next_eligible_at(&conn, &policy, now).unwrap(),
            Some(now + 3600 + 86_401)
        );
    }
    #[test]
    fn missing_timezone_and_quiet_period_never_use_the_server_clock() {
        let (conn, mut policy) = fixture(None);
        let now = clock("2026-11-15T14:00:00Z");
        assert_eq!(
            deferred_reason(&conn, &policy, now).unwrap(),
            Some(DeliveryDeferredReason::TimezoneRequired)
        );
        assert_eq!(next_eligible_at(&conn, &policy, now).unwrap(), None);
        policy.timezone = Some("Asia/Shanghai".into());
        assert_eq!(
            deferred_reason(&conn, &policy, now).unwrap(),
            Some(DeliveryDeferredReason::QuietHours)
        );
        assert_eq!(
            next_eligible_at(&conn, &policy, now).unwrap(),
            Some(clock("2026-11-16T00:00:00Z"))
        );
    }
    #[test]
    fn equal_hours_mean_all_day_quiet_and_daytime_intervals_are_supported() {
        let (conn, mut policy) = fixture(Some("Asia/Shanghai"));
        let now = clock("2026-11-15T03:00:00Z");
        policy.quiet_start_hour = 11;
        policy.quiet_end_hour = 11;
        assert_eq!(next_eligible_at(&conn, &policy, now).unwrap(), None);
        policy.quiet_end_hour = 12;
        assert_eq!(
            next_eligible_at(&conn, &policy, now).unwrap(),
            Some(clock("2026-11-15T04:00:00Z"))
        );
    }
}
