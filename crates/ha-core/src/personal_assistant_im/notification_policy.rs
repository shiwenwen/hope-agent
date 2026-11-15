use anyhow::Result;
use chrono_tz::Tz;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

pub const QUIET_START_HOUR: u32 = 22;
pub const QUIET_END_HOUR: u32 = 8;
pub const DAILY_LIMIT: usize = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationPolicy {
    pub timezone: Option<String>,
    pub quiet_start_hour: u32,
    pub quiet_end_hour: u32,
    pub daily_limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryDeferredReason {
    TimezoneRequired,
    QuietHours,
    DailyLimit,
}

impl NotificationPolicy {
    pub(super) fn from_timezone(timezone: Option<&str>) -> Self {
        Self {
            timezone: timezone
                .and_then(|name| name.parse::<Tz>().ok())
                .map(|timezone| timezone.name().to_owned()),
            quiet_start_hour: QUIET_START_HOUR,
            quiet_end_hour: QUIET_END_HOUR,
            daily_limit: DAILY_LIMIT,
        }
    }
}

/// The owner's explicit profile is the sole timezone source. A missing,
/// unreadable or invalid profile never falls back to the server's timezone.
pub fn notification_policy() -> NotificationPolicy {
    let profile = crate::user_config::load_user_config().ok();
    NotificationPolicy::from_timezone(
        profile
            .as_ref()
            .and_then(|profile| profile.timezone.as_deref()),
    )
}

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS assistant_im_notification_attempts (
            delivery_id TEXT PRIMARY KEY,
            attempted_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_assistant_im_notification_attempt_time
         ON assistant_im_notification_attempts(attempted_at);",
    )?;
    Ok(())
}

/// Called only in the same IMMEDIATE transaction as pending -> submitting.
/// No foreign key: deleting a connection, delivery or source cannot refund an
/// attempted interruption. Unknown and proven-unsent attempts stay charged.
pub(super) fn reserve_attempt(conn: &Connection, delivery_id: &str, now: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO assistant_im_notification_attempts(delivery_id,attempted_at) VALUES (?1,?2)",
        params![delivery_id, now],
    )?;
    Ok(())
}
