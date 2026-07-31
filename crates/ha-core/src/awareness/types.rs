//! Value types for behavior awareness.

use serde::{Deserialize, Serialize};

/// Session kind (for rendering labels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Regular,
    Cron,
    Channel,
    Subagent,
}

impl SessionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionKind::Regular => "regular",
            SessionKind::Cron => "cron",
            SessionKind::Channel => "channel",
            SessionKind::Subagent => "subagent",
        }
    }
}

/// Liveness state of a session at snapshot time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityState {
    /// Touched within `active_window_secs`.
    Active,
    /// Touched within the last hour but not in the active window.
    Recent,
    /// Older than an hour but still within `lookback_hours`.
    Older,
}

/// One collected entry describing a peer session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AwarenessEntry {
    pub session_id: String,
    pub title: String,
    pub agent_id: String,
    pub agent_name: Option<String>,
    pub session_kind: SessionKind,
    pub updated_at: String,
    /// Seconds ago (computed at collection time). Used by render.rs.
    pub age_secs: i64,
    pub activity: ActivityState,
    // ── Facet-derived (optional) ──
    pub brief_summary: Option<String>,
    pub underlying_goal: Option<String>,
    pub outcome: Option<String>,
    pub goal_categories: Vec<String>,
    // ── Fallback when no facet is cached ──
    pub fallback_preview: Option<String>,
}

/// Snapshot produced by `collect_entries()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AwarenessSnapshot {
    pub entries: Vec<AwarenessEntry>,
    pub active_count: usize,
    pub generated_at: String,
}

/// Why (or why not) the suffix was refreshed on this turn. Useful for logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshReason {
    DirtyBit,
    TimeWindow,
    SemanticHint,
    Forced,
    /// Reused previous suffix.
    Cached,
    /// Feature disabled or no candidates.
    None,
}

impl RefreshReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            RefreshReason::DirtyBit => "dirty_bit",
            RefreshReason::TimeWindow => "time_window",
            RefreshReason::SemanticHint => "semantic_hint",
            RefreshReason::Forced => "forced",
            RefreshReason::Cached => "cached",
            RefreshReason::None => "none",
        }
    }
}

/// `recap_facets` 里被 awareness 候选行富化用到的**那四个字段**的只读视图。
///
/// 刻意不是 `SessionFacet` 本体：facet 的完整结构（满意度 / 摩擦计数 /
/// 用户指令等十余项）是 recap 报告的领域模型，awareness 只读这四项。让
/// kernel 认识一个窄视图，比把整张表的 wire 类型拖下来更省，也不会在
/// ha-dash 演进 facet 结构时反复牵动 kernel。
#[derive(Debug, Clone, Default)]
pub struct SessionFacetView {
    pub brief_summary: String,
    pub underlying_goal: String,
    /// `Outcome` 的小写 Debug 名（与迁移前 `format!("{:?}", …).to_lowercase()`
    /// 逐字相同）——kernel 不认识 `Outcome` 枚举本体。
    pub outcome: String,
    pub goal_categories: Vec<String>,
}
