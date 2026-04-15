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
pub mod renderer;
pub mod report;
pub mod sections;
pub mod types;

pub use db::RecapDb;
pub use renderer::render_html;
pub use report::{build_analysis_agent, generate_report, RecapContext};
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
            crate::app_warn!(
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
        Ok(n) => crate::app_info!(
            "recap",
            "retention",
            "Purged {} expired facet row(s) (retention={} days)",
            n,
            retention_days
        ),
        Err(e) => crate::app_warn!(
            "recap",
            "retention",
            "Facet retention sweep failed: {}",
            e
        ),
    }
}

/// Spawn a background task that purges expired recap facets once at startup
/// and then once per day. Skipped entirely when `cache_retention_days == 0`,
/// so a fully-off config doesn't leave a permanent 24h ticker doing nothing.
pub fn spawn_facet_retention_loop() {
    let retention_days = crate::config::cached_config().recap.cache_retention_days;
    if retention_days == 0 {
        return;
    }

    tokio::spawn(async move {
        tokio::task::spawn_blocking(move || run_facet_retention_once(retention_days));

        let mut ticker =
            tokio::time::interval(std::time::Duration::from_secs(crate::SECS_PER_DAY));
        ticker.tick().await; // interval fires immediately on first tick; consume it
        loop {
            ticker.tick().await;
            let days = crate::config::cached_config().recap.cache_retention_days;
            if days == 0 {
                continue;
            }
            tokio::task::spawn_blocking(move || run_facet_retention_once(days));
        }
    });
}
