//! Recap — deep per-session semantic analysis + aggregated coaching reports.
//!
//! Extracts qualitative facets from each session via the analysis agent,
//! combines them with quantitative stats from the dashboard queries, and
//! produces a report with AI-generated sections that can be viewed in
//! the Dashboard, streamed into chat, or exported as standalone HTML.

pub mod aggregate;
pub mod api;
pub mod db;
pub mod facets;
mod i18n;
pub mod renderer;
pub mod report;
pub mod sections;
pub mod slash;
pub mod types;

pub use db::RecapDb;
pub use renderer::render_html;
pub use report::{generate_report, RecapContext};
pub use types::{
    AiSection, FacetSummary, FrictionCounts, GenerateMode, Outcome, QuantitativeStats,
    RecapFilters, RecapProgress, RecapReport, RecapReportSummary, ReportMeta, SessionFacet,
    RECAP_SCHEMA_VERSION,
};

/// Open the recap DB and purge facet rows older than `retention_days`.
/// No-op when `retention_days == 0`.
fn run_facet_retention_once(retention_days: u32) {
    if retention_days == 0 {
        return;
    }
    let db = match RecapDb::open_default() {
        Ok(db) => db,
        Err(e) => {
            app_warn!(
                "recap",
                "retention",
                "Failed to open recap DB for retention sweep: {}",
                e
            );
            return;
        }
    };
    match db.purge_old_facets(retention_days) {
        Ok(0) => {}
        Ok(n) => app_info!(
            "recap",
            "retention",
            "Purged {} expired facet row(s) (retention={} days)",
            n,
            retention_days
        ),
        Err(e) => app_warn!("recap", "retention", "Facet retention sweep failed: {}", e),
    }
}

/// Spawn a background task that purges expired recap facets once at startup
/// and then once per day. Skipped entirely when `cache_retention_days == 0`,
/// so a fully-off config doesn't leave a permanent 24h ticker doing nothing.
pub fn spawn_facet_retention_loop() {
    let retention_days = ha_core::config::cached_config().recap.cache_retention_days;
    if retention_days == 0 {
        return;
    }

    tokio::spawn(async move {
        tokio::task::spawn_blocking(move || run_facet_retention_once(retention_days));

        let mut ticker =
            tokio::time::interval(std::time::Duration::from_secs(ha_core::SECS_PER_DAY));
        ticker.tick().await; // interval fires immediately on first tick; consume it
        loop {
            ticker.tick().await;
            let days = ha_core::config::cached_config().recap.cache_retention_days;
            if days == 0 {
                continue;
            }
            tokio::task::spawn_blocking(move || run_facet_retention_once(days));
        }
    });
}

// ── awareness 的 facet 查询实现（kernel 钩子，`wire()` 注册）────────
//
// awareness 只读 facet 的四个字段，故经 kernel 的窄视图 `SessionFacetView`
// 回传，不把 `SessionFacet` 本体拖进 kernel。
//
// **返回值与迁移前逐位相同，但失败路径的代价被放大了**（如实登记）：迁移前
// `collect_entries` 在循环**外**取一次 guard 并整段持有，`RecapDb::open_default()`
// 每次刷新最多重试 1 次；现在钩子在**逐候选会话的循环体内**被调，recap.db
// 打不开时一次刷新会尝试 N 次开库（N = 候选会话数）外加 N 次锁进出。返回
// None 的语义不变、调用方的 fallback_preview 分支不变，只是开库失败时更费。
// 要消掉这个放大得让钩子签名收一批 session_id（awareness 侧也要改），本刀
// 不为一条冷路径动接口。

/// 惰性缓存的 `RecapDb` 连接。`None` = 上次打开失败，下次再试。
static FACET_DB: std::sync::Mutex<Option<db::RecapDb>> = std::sync::Mutex::new(None);

/// [`ha_core::awareness::SessionFacetLookup`] 的实现。无 facet / 打不开
/// 库 / 查询出错一律 `None`——调用方走它既有的 fallback preview 分支。
pub fn facet_view_for_session(
    session_id: &str,
) -> Option<ha_core::awareness::types::SessionFacetView> {
    let mut guard = FACET_DB.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = db::RecapDb::open_default().ok();
    }
    let facet = guard
        .as_ref()
        .and_then(|db| db.get_latest_facet(session_id).ok().flatten())?;
    Some(ha_core::awareness::types::SessionFacetView {
        brief_summary: facet.brief_summary,
        underlying_goal: facet.underlying_goal,
        // 与迁移前 `format!("{:?}", f.outcome).to_lowercase()` 逐字相同。
        outcome: format!("{:?}", facet.outcome).to_lowercase(),
        goal_categories: facet.goal_categories,
    })
}
