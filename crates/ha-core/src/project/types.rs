//! Project types.
//!
//! A `Project` is an optional container that groups multiple sessions so they
//! can share memories (`MemoryScope::Project`) and a working directory.
//! Project instructions are not stored in this record: the project root's
//! `AGENTS.md` is the sole source of truth. Sessions with `project_id = NULL`
//! keep the pre-project behavior and are unaffected.

use serde::{Deserialize, Serialize};

use crate::session::SessionMeta;

// ── Project ─────────────────────────────────────────────────────

/// Persisted project record.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional project logo stored as a `data:image/...;base64,...` URL.
    /// Rendered in the sidebar row and overview header when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// When set, new sessions created inside this project default to this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent_id: Option<String>,
    /// When set, new sessions created inside this project default to this model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model_id: Option<String>,
    /// Default working directory for sessions in this project. Resolved at
    /// system-prompt build time as the fallback when the session itself has
    /// no `working_dir` set (session-level overrides project-level).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// Unix milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
    /// Sidebar sort key. Lower values render earlier.
    #[serde(default)]
    pub sort_order: i64,
    #[serde(default)]
    pub archived: bool,
}

impl Project {
    /// Human-readable label used by pickers and IM message bodies.
    pub fn display_label(&self) -> String {
        self.name.clone()
    }
}

/// Project with counts aggregated from related tables, for listing / UI use.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMeta {
    #[serde(flatten)]
    pub project: Project,
    pub session_count: u32,
    pub unread_count: u32,
}

/// Read-only AGENTS.md status shown on the project overview dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInstructionsStats {
    pub path: String,
    pub line_count: u32,
    pub size_bytes: u64,
    pub empty: bool,
}

/// Aggregated, user-facing project overview. Optional filesystem / memory
/// metrics fail independently so one unavailable store never blanks the page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOverviewSummary {
    pub session_count: u32,
    pub recent_sessions: Vec<SessionMeta>,
    pub auto_memory_topic_count: Option<u32>,
    pub active_claim_count: Option<u32>,
    pub instructions: Option<ProjectInstructionsStats>,
}

// ── Input DTOs ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectInput {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub default_agent_id: Option<String>,
    #[serde(default)]
    pub default_model_id: Option<String>,
    /// Optional default working directory for sessions in this project.
    /// Empty string is normalized to `NULL` by the DB layer.
    #[serde(default)]
    pub working_dir: Option<String>,
}

/// Patch DTO. `None` means "do not change this field". Clearing a field is
/// expressed by passing `Some(None)` at the JSON level via `serde_with`'s
/// double-option pattern.
///
/// Kept simple here: callers that need to clear a field should pass an empty
/// string, which is normalized to `NULL` inside [`ProjectDB::update`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProjectInput {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub default_agent_id: Option<String>,
    #[serde(default)]
    pub default_model_id: Option<String>,
    /// Patch the project default working directory. Empty string clears it.
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub archived: Option<bool>,
}
