//! Learning analytics queries + event emission helpers (Phase B'4).
//!
//! The DDL + insert/prune helpers live on `SessionDB` (so `learning_events`
//! shares the session DB connection). This module wraps them with:
//!   - `emit(kind, session_id, ref_id, meta)` — cheap fire-and-forget
//!     call used by skill CRUD / recall summary / auto-review pipelines.
//!     Resolves the global SessionDB lazily so callers don't have to plumb
//!     an Arc through their signatures.
//!   - Aggregate queries that power the Dashboard "Learning" tab.

use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::session::SessionDB;

// ── Event kinds (stable strings stored in learning_events.kind) ──
pub const EVT_SKILL_CREATED: &str = "skill_created";
pub const EVT_SKILL_PATCHED: &str = "skill_patched";
pub const EVT_SKILL_ACTIVATED: &str = "skill_activated";
pub const EVT_SKILL_DISCARDED: &str = "skill_discarded";
pub const EVT_SKILL_USED: &str = "skill_used";
pub const EVT_RECALL_HIT: &str = "recall_hit";
pub const EVT_RECALL_SUMMARY_USED: &str = "recall_summary_used";

/// Best-effort emitter. Silently no-ops if the session DB hasn't been
/// initialized (e.g. in unit tests for subsystems that don't need one).
///
/// Dispatches the INSERT onto `spawn_blocking` when we're inside a Tokio
/// runtime so the caller (often a hot path like `tool_recall_memory` or a
/// skill CRUD op) doesn't wait on the SessionDB writer Mutex. Falls back to
/// a sync call outside an async context (e.g. from a blocking worker).
pub fn emit(
    kind: &str,
    session_id: Option<&str>,
    ref_id: Option<&str>,
    meta: Option<&serde_json::Value>,
) {
    let Some(db) = crate::get_session_db().cloned() else {
        return;
    };
    let kind = kind.to_string();
    let session_id = session_id.map(str::to_string);
    let ref_id = ref_id.map(str::to_string);
    let meta = meta.cloned();
    let write = move || {
        db.record_learning_event(
            &kind,
            session_id.as_deref(),
            ref_id.as_deref(),
            meta.as_ref(),
        );
    };
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn_blocking(write);
        }
        Err(_) => write(),
    }
}

// ── Aggregate queries ───────────────────────────────────────────

/// Summary for the "Learning" overview card: counts bucketed by event kind
/// over the given window, with source breakdowns for skill creations.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LearningOverview {
    pub window_days: u32,
    pub auto_created_skills: u64,
    pub user_created_skills: u64,
    pub skills_activated: u64,
    pub skills_patched: u64,
    pub skills_discarded: u64,
    pub skills_used: u64,
    pub recall_hits: u64,
    pub recall_summary_used: u64,
    pub profile_memories: u64,
}

/// One row in the skill-events timeline: `ts` is unix seconds, `kind` is
/// one of the `EVT_SKILL_*` constants.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelinePoint {
    pub ts: i64,
    pub kind: String,
    pub skill_id: Option<String>,
    pub source: Option<String>,
}

/// Top-N skill by use count within the window.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillUsage {
    pub skill_id: String,
    pub used_count: u64,
    pub last_used_ts: Option<i64>,
    pub created_source: Option<String>,
}

pub fn query_learning_overview(db: &SessionDB, window_days: u32) -> Result<LearningOverview> {
    let cutoff = crate::util::epoch_cutoff_secs(window_days);
    let conn = db.conn.lock().map_err(|e| anyhow::anyhow!("Lock: {}", e))?;

    let mut overview = LearningOverview {
        window_days,
        ..Default::default()
    };

    // Generic count-by-kind scan in a single query.
    let mut stmt = conn.prepare(
        "SELECT kind, COUNT(*) FROM learning_events
         WHERE ts >= ?1 GROUP BY kind",
    )?;
    let rows = stmt.query_map(params![cutoff], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
    })?;
    for row in rows {
        let (kind, count) = row?;
        match kind.as_str() {
            EVT_SKILL_PATCHED => overview.skills_patched = count,
            EVT_SKILL_ACTIVATED => overview.skills_activated = count,
            EVT_SKILL_DISCARDED => overview.skills_discarded = count,
            EVT_SKILL_USED => overview.skills_used = count,
            EVT_RECALL_HIT => overview.recall_hits = count,
            EVT_RECALL_SUMMARY_USED => overview.recall_summary_used = count,
            _ => {}
        }
    }

    // Breakdown skill_created by source (auto-review vs user). Push the
    // JSON extraction into SQLite with `json_extract` so we get two rows
    // instead of one row-per-event; the `COALESCE` maps missing sources
    // (pre-B'4 data) to "user" to match legacy behavior.
    let mut stmt2 = conn.prepare(
        "SELECT COALESCE(json_extract(meta_json, '$.source'), 'user') AS src,
                COUNT(*)
         FROM learning_events
         WHERE kind = ?1 AND ts >= ?2
         GROUP BY src",
    )?;
    let rows = stmt2.query_map(params![EVT_SKILL_CREATED, cutoff], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
    })?;
    for row in rows {
        let (src, count) = row?;
        if src == "auto-review" {
            overview.auto_created_skills = count;
        } else {
            overview.user_created_skills += count;
        }
    }

    // Reflective (profile-tagged) memories — the profile count comes from
    // the memories table itself rather than the event stream (we don't
    // emit a learning_event for every extraction). Query via the memory
    // backend's SQLite connection when available.
    overview.profile_memories = profile_memory_count(window_days).unwrap_or(0);

    Ok(overview)
}

/// Skill-lifecycle timeline for the given window, newest last for easy
/// charting.
pub fn query_skill_timeline(db: &SessionDB, window_days: u32) -> Result<Vec<TimelinePoint>> {
    let cutoff = crate::util::epoch_cutoff_secs(window_days);
    let conn = db.conn.lock().map_err(|e| anyhow::anyhow!("Lock: {}", e))?;
    let mut stmt = conn.prepare(
        "SELECT ts, kind, ref_id, meta_json FROM learning_events
         WHERE ts >= ?1 AND kind IN (
           'skill_created','skill_patched','skill_activated','skill_discarded'
         )
         ORDER BY ts ASC",
    )?;
    let rows = stmt.query_map(params![cutoff], |row| {
        let ts: i64 = row.get(0)?;
        let kind: String = row.get(1)?;
        let skill_id: Option<String> = row.get(2)?;
        let meta: Option<String> = row.get(3)?;
        let source = meta
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v| v.get("source").and_then(|s| s.as_str()).map(String::from));
        Ok(TimelinePoint {
            ts,
            kind,
            skill_id,
            source,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Top skills by `skill_used` count within the window.
pub fn query_top_skills(db: &SessionDB, window_days: u32, limit: usize) -> Result<Vec<SkillUsage>> {
    let cutoff = crate::util::epoch_cutoff_secs(window_days);
    let conn = db.conn.lock().map_err(|e| anyhow::anyhow!("Lock: {}", e))?;
    let mut stmt = conn.prepare(
        "SELECT ref_id, COUNT(*) AS c, MAX(ts) AS last_ts
         FROM learning_events
         WHERE ts >= ?1 AND kind = ?2 AND ref_id IS NOT NULL
         GROUP BY ref_id
         ORDER BY c DESC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![cutoff, EVT_SKILL_USED, limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)? as u64,
            row.get::<_, Option<i64>>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (skill_id, used_count, last_used_ts) = row?;
        out.push(SkillUsage {
            skill_id,
            used_count,
            last_used_ts,
            created_source: None,
        });
    }
    Ok(out)
}

/// Recall-hit vs summary-used tallies so the Dashboard can render a donut.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecallStats {
    pub hits: u64,
    pub summarized: u64,
    pub window_days: u32,
}

pub fn query_recall_stats(db: &SessionDB, window_days: u32) -> Result<RecallStats> {
    let cutoff = crate::util::epoch_cutoff_secs(window_days);
    let conn = db.conn.lock().map_err(|e| anyhow::anyhow!("Lock: {}", e))?;
    let hits: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM learning_events WHERE ts >= ?1 AND kind = ?2",
            params![cutoff, EVT_RECALL_HIT],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let summarized: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM learning_events WHERE ts >= ?1 AND kind = ?2",
            params![cutoff, EVT_RECALL_SUMMARY_USED],
            |row| row.get(0),
        )
        .unwrap_or(0);
    Ok(RecallStats {
        hits: hits as u64,
        summarized: summarized as u64,
        window_days,
    })
}

/// Count memories tagged `profile` created within the window. Reads the
/// memories SQLite directly via the backend; gracefully returns None when
/// the backend isn't initialized.
fn profile_memory_count(window_days: u32) -> Option<u64> {
    let backend = crate::get_memory_backend()?;
    backend.count_profile_memories(window_days).ok()
}
