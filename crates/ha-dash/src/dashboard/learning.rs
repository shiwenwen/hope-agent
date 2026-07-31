//! Learning analytics **消费面**——Dashboard「Learning」标签页的只读聚合。
//!
//! 事件**发布面**（`emit` + `EVT_*` 常量）在 [`ha_core::learning_events`]
//! （crate-split 破环：生产者遍布 kernel / skills / knowledge / ha-mcp
//! 四层，发布面留这里会让它们全部反向依赖 dashboard）。本模块与发布面
//! 之间没有代码耦合，只共享 `learning_events` 表名与 kind 字符串；DDL /
//! INSERT / prune / 会话级联删除都在 [`ha_core::session::SessionDB`]。
//!
//! 原路径 `dashboard::learning::{emit, EVT_*}` 保留再导出，调用点
//! 不受影响。

use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};

// 发布面原路径兼容再导出（定义处 `ha_core::learning_events`）。
pub use ha_core::learning_events::{
    emit, EVT_MCP_TOOL_CALLED, EVT_MCP_TOOL_FAILED, EVT_RECALL_HIT, EVT_RECALL_SUMMARY_USED,
    EVT_SKILL_ACTIVATED, EVT_SKILL_CREATED, EVT_SKILL_DISCARDED, EVT_SKILL_PATCHED, EVT_SKILL_USED,
};

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

pub fn query_learning_overview(window_days: u32) -> Result<LearningOverview> {
    let cutoff = ha_core::util::epoch_cutoff_secs(window_days);
    let conn = crate::db::read_conn()?;

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
pub fn query_skill_timeline(window_days: u32) -> Result<Vec<TimelinePoint>> {
    let cutoff = ha_core::util::epoch_cutoff_secs(window_days);
    let conn = crate::db::read_conn()?;
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
pub fn query_top_skills(window_days: u32, limit: usize) -> Result<Vec<SkillUsage>> {
    let cutoff = ha_core::util::epoch_cutoff_secs(window_days);
    let conn = crate::db::read_conn()?;
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

pub fn query_recall_stats(window_days: u32) -> Result<RecallStats> {
    let cutoff = ha_core::util::epoch_cutoff_secs(window_days);
    let conn = crate::db::read_conn()?;
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
    let backend = ha_core::get_memory_backend()?;
    backend.count_profile_memories(window_days).ok()
}
