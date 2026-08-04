//! Behavior Awareness
//!
//! Gives each chat session a short, dynamically-refreshed markdown block
//! describing what the user is doing in other sessions right now.
//!
//! Two data paths:
//!   1. **Structured** (zero LLM cost): reads `session_facets` / `sessions` /
//!      in-memory `ActiveSessionRegistry` and renders a compact list.
//!   2. **LLM Digest** (opt-in): uses `AssistantAgent::side_query` to turn the
//!      candidates into a concrete "what is the user actually doing" paragraph.
//!      Runs async, never blocks the current chat turn.
//!
//! The suffix lives outside the cached system-prompt prefix so prompt-cache
//! hits on the static prefix are preserved even when the suffix changes.

pub mod collect;
pub mod config;
pub mod dirty;
pub mod llm_digest;
pub mod peek_tool;
pub mod registry;
pub mod render;
pub mod session;
pub mod types;

pub use config::{resolve_for_session, AwarenessConfig, AwarenessMode, LlmExtractionConfig};
pub use dirty::{mark_all_except, on_other_session_activity, take_dirty};
pub use peek_tool::{peek_sessions_schema, run_peek_sessions};
pub use registry::{active_since, active_snapshot, touch_active_session};
pub use session::SessionAwareness;
pub use types::{ActivityState, AwarenessEntry, AwarenessSnapshot, RefreshReason, SessionKind};

// ── facet 查询钩子（ha-dash 装配期注册）─────────────────────────
//
// awareness 的候选行富化要读 `recap_facets`，但 recap 已随 ha-dash 迁出，
// kernel 不能反向依赖它。倒转为钩子：ha-dash 的 `wire()` 注册查询实现。
//
// **未注册即 `None`**，`collect::collect_entries` 走它既有的
// `fallback_preview` 分支——**返回值**与迁移前 `RecapDb::open_default()` 打
// 不开时相同，不是新增的降级路径。（钩子实现侧的开库重试次数由每次刷新一次
// 变成每候选会话一次，见 ha-dash `recap::facet_view_for_session` 的登记。）

/// 按 session id 查最近一条 facet 视图；无 facet / 查询失败均返回 `None`。
pub type SessionFacetLookup = fn(&str) -> Option<types::SessionFacetView>;

static SESSION_FACET_LOOKUP: std::sync::OnceLock<SessionFacetLookup> = std::sync::OnceLock::new();

/// 装配期注册 facet 查询实现。重复注册返回 `Err`。
pub fn register_session_facet_lookup(
    lookup: SessionFacetLookup,
) -> Result<(), crate::AlreadyRegistered> {
    SESSION_FACET_LOOKUP
        .set(lookup)
        .map_err(|_| crate::AlreadyRegistered("awareness session-facet lookup"))
}

/// 查一条 facet 视图。未装配返回 `None`（等价于「该会话没有 facet」）。
pub fn session_facet_view(session_id: &str) -> Option<types::SessionFacetView> {
    (SESSION_FACET_LOOKUP.get()?)(session_id)
}
