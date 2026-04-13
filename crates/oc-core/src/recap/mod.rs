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
