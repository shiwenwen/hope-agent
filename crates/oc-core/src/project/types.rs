//! Project types.
//!
//! A `Project` is an optional container that groups multiple sessions so they
//! can share memories (`MemoryScope::Project`), custom instructions, and
//! uploaded files. Sessions with `project_id = NULL` keep the pre-project
//! behavior and are unaffected.

use serde::{Deserialize, Serialize};

// ── Project ─────────────────────────────────────────────────────

/// Persisted project record.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Custom instructions appended to the system prompt for every session in the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// When set, new sessions created inside this project default to this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent_id: Option<String>,
    /// When set, new sessions created inside this project default to this model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model_id: Option<String>,
    /// Unix milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub archived: bool,
}

/// Project with counts aggregated from related tables, for listing / UI use.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMeta {
    #[serde(flatten)]
    pub project: Project,
    pub session_count: u32,
    pub file_count: u32,
    pub memory_count: u32,
}

// ── Input DTOs ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectInput {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub default_agent_id: Option<String>,
    #[serde(default)]
    pub default_model_id: Option<String>,
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
    pub instructions: Option<String>,
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub default_agent_id: Option<String>,
    #[serde(default)]
    pub default_model_id: Option<String>,
    #[serde(default)]
    pub archived: Option<bool>,
}

// ── Project Files ───────────────────────────────────────────────

/// Metadata row for a file uploaded to a project. The physical bytes live
/// under `~/.opencomputer/projects/{project_id}/files/`, and extracted text
/// (if any) under `~/.opencomputer/projects/{project_id}/extracted/`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFile {
    pub id: String,
    pub project_id: String,
    /// Display name (user-editable, defaults to `original_filename`).
    pub name: String,
    pub original_filename: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub size_bytes: i64,
    /// Stored path relative to `paths::projects_dir()`.
    pub file_path: String,
    /// Stored extracted-text path relative to `paths::projects_dir()`.
    /// `None` when the file is binary or extraction failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted_path: Option<String>,
    /// Character count of the extracted text, used for inline-budget math.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted_chars: Option<i64>,
    /// Optional LLM-generated one-liner summary (not populated in the initial version).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}
