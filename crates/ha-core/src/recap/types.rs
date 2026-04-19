use serde::{Deserialize, Serialize};

use crate::dashboard::{
    DashboardCostTrend, DashboardFilter, DashboardHeatmap, DashboardHourlyDistribution,
    HealthBreakdown, ModelEfficiency, OverviewStatsWithDelta, TopSession,
};

/// Filter passed to recap report generation. Reuses Dashboard's filter shape.
pub type RecapFilters = DashboardFilter;

/// How a report should be generated.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum GenerateMode {
    /// Analyse only sessions touched since the previous report.
    /// Falls back to last `default_range_days` if no prior report exists.
    Incremental,
    /// Analyse all sessions matching the filters.
    Full {
        #[serde(default)]
        filters: RecapFilters,
    },
}

/// Outcome category extracted from a session.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    FullyAchieved,
    MostlyAchieved,
    Partial,
    Failed,
    #[default]
    Unclear,
}

/// Friction taxonomy (counts per session).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrictionCounts {
    #[serde(default)]
    pub tool_errors: u32,
    #[serde(default)]
    pub misunderstanding: u32,
    #[serde(default)]
    pub repetition: u32,
    #[serde(default)]
    pub user_correction: u32,
    #[serde(default)]
    pub stuck: u32,
    #[serde(default)]
    pub other: u32,
}

/// Per-session semantic facets extracted by the analysis agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionFacet {
    pub session_id: String,
    pub underlying_goal: String,
    #[serde(default)]
    pub goal_categories: Vec<String>,
    #[serde(default)]
    pub outcome: Outcome,
    #[serde(default)]
    pub user_satisfaction: Option<u8>,
    #[serde(default)]
    pub agent_helpfulness: Option<u8>,
    #[serde(default)]
    pub session_type: String,
    #[serde(default)]
    pub friction_counts: FrictionCounts,
    #[serde(default)]
    pub friction_detail: Vec<String>,
    #[serde(default)]
    pub primary_success: Option<String>,
    pub brief_summary: String,
    #[serde(default)]
    pub user_instructions: Vec<String>,
}

/// Aggregated histograms / top-N derived from facets.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FacetSummary {
    pub total_facets: u32,
    pub goal_histogram: Vec<(String, u32)>,
    pub outcome_distribution: Vec<(String, u32)>,
    pub session_type_distribution: Vec<(String, u32)>,
    pub friction_top: Vec<(String, u32)>,
    pub satisfaction_distribution: Vec<(u8, u32)>,
    pub repeat_user_instructions: Vec<(String, u32)>,
    pub success_examples: Vec<String>,
    pub friction_examples: Vec<String>,
}

/// Quantitative stats sourced from existing dashboard queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuantitativeStats {
    pub overview: OverviewStatsWithDelta,
    pub health: HealthBreakdown,
    pub cost_trend: DashboardCostTrend,
    pub heatmap: DashboardHeatmap,
    pub hourly: DashboardHourlyDistribution,
    pub top_sessions: Vec<TopSession>,
    pub model_efficiency: Vec<ModelEfficiency>,
}

/// One AI-generated report section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSection {
    pub key: String,
    pub title: String,
    pub markdown: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportMeta {
    pub id: String,
    pub title: String,
    /// ISO-8601 / SQLite-compatible date strings to align with DashboardFilter.
    pub range_start: String,
    pub range_end: String,
    pub session_count: u32,
    pub generated_at: String,
    pub analysis_model: String,
    pub filters: RecapFilters,
    /// Compatibility/migration version of the report shape.
    pub schema_version: u32,
}

/// Top-level recap report stored to disk and returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecapReport {
    pub meta: ReportMeta,
    pub quantitative: QuantitativeStats,
    pub facet_summary: FacetSummary,
    pub sections: Vec<AiSection>,
}

/// Lightweight summary used by history listings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecapReportSummary {
    pub id: String,
    pub title: String,
    pub range_start: String,
    pub range_end: String,
    pub session_count: u32,
    pub generated_at: String,
    pub analysis_model: String,
    pub html_path: Option<String>,
}

/// Streamed progress events for the generation pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "camelCase")]
pub enum RecapProgress {
    Started {
        report_id: String,
        total_sessions: u32,
    },
    ExtractingFacets {
        completed: u32,
        total: u32,
    },
    AggregatingDashboard,
    GeneratingSections {
        completed: u32,
        total: u32,
    },
    Persisting,
    Done {
        report_id: String,
    },
    Failed {
        report_id: String,
        message: String,
    },
}

/// Current schema version for `session_facets` and `recap_reports` payloads.
pub const RECAP_SCHEMA_VERSION: u32 = 1;
