//! Types for the Knowledge Layer-2 autonomous maintenance (WS6): proposal kinds,
//! the concrete file action each carries, the persisted proposal row, and the
//! per-cycle report. Proposals are **drafts** — nothing touches a user's notes
//! until the owner approves one in the review queue.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The eight maintenance task kinds (design: Layer 2 roadmap).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalKind {
    /// A note's body mentions another note's title but has no `[[ ]]` link.
    AutoLink,
    /// A note with no inbound/outbound resolved links — link it from a relative.
    OrphanRescue,
    /// A note missing basic frontmatter (title / created / tags scaffold).
    FrontmatterFill,
    /// Two near-duplicate notes — merge into one.
    DedupMerge,
    /// A frequently-referenced broken `[[ ]]` target — create the missing note.
    KnowledgeGap,
    /// Suggest tags for an untagged note (LLM).
    AutoTag,
    /// Build / refresh a Map-of-Content hub for a busy topic or tag (LLM).
    MocUpkeep,
    /// Distil scattered memories into a permanent topic note (LLM).
    MemoryToNote,
}

impl ProposalKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProposalKind::AutoLink => "auto_link",
            ProposalKind::OrphanRescue => "orphan_rescue",
            ProposalKind::FrontmatterFill => "frontmatter_fill",
            ProposalKind::DedupMerge => "dedup_merge",
            ProposalKind::KnowledgeGap => "knowledge_gap",
            ProposalKind::AutoTag => "auto_tag",
            ProposalKind::MocUpkeep => "moc_upkeep",
            ProposalKind::MemoryToNote => "memory_to_note",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "auto_link" => ProposalKind::AutoLink,
            "orphan_rescue" => ProposalKind::OrphanRescue,
            "frontmatter_fill" => ProposalKind::FrontmatterFill,
            "dedup_merge" => ProposalKind::DedupMerge,
            "knowledge_gap" => ProposalKind::KnowledgeGap,
            "auto_tag" => ProposalKind::AutoTag,
            "moc_upkeep" => ProposalKind::MocUpkeep,
            "memory_to_note" => ProposalKind::MemoryToNote,
            _ => return None,
        })
    }

    /// Whether generating this kind needs an LLM side_query (cost gate). The
    /// deterministic kinds (auto-link, orphan, frontmatter, dedup, knowledge-gap
    /// stub) run from a `spawn_blocking` index scan with no model call.
    pub fn is_llm(&self) -> bool {
        matches!(
            self,
            ProposalKind::AutoTag | ProposalKind::MocUpkeep | ProposalKind::MemoryToNote
        )
    }
}

/// Lifecycle of a proposal in the review queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    /// Awaiting owner review (default).
    Draft,
    /// Approved and successfully written to the KB.
    Applied,
    /// Owner declined.
    Rejected,
    /// Approved but the write failed (see `error`).
    Failed,
}

impl ProposalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProposalStatus::Draft => "draft",
            ProposalStatus::Applied => "applied",
            ProposalStatus::Rejected => "rejected",
            ProposalStatus::Failed => "failed",
        }
    }
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "draft" => ProposalStatus::Draft,
            "applied" => ProposalStatus::Applied,
            "rejected" => ProposalStatus::Rejected,
            "failed" => ProposalStatus::Failed,
            _ => return None,
        })
    }
}

/// The concrete, owner-plane file action a proposal applies. Kept to four shapes
/// so the applier ([`super::apply`]) has one arm each; every generator maps onto
/// one of these.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ProposalAction {
    /// Append a `[[to_ref]]` link to `from_path` (auto-link, orphan rescue).
    AppendLink { from_path: String, to_ref: String },
    /// Merge `props` into `path`'s YAML frontmatter (frontmatter fill, auto-tag).
    SetFrontmatter {
        path: String,
        props: Map<String, Value>,
    },
    /// Create (or overwrite) a note (knowledge gap, MOC upkeep, memory→note).
    CreateNote {
        path: String,
        content: String,
        #[serde(default)]
        overwrite: bool,
    },
    /// Replace `keep_path` with merged content and delete `removes` (dedup). Each
    /// path carries its generation-time content hash so the applier can refuse if
    /// any note changed since the proposal was made (the merged content + the
    /// delete would otherwise silently clobber newer edits).
    MergeNotes {
        keep_path: String,
        keep_expected_hash: String,
        keep_content: String,
        removes: Vec<MergeRemove>,
    },
}

/// A note to delete in a merge, with its generation-time content hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeRemove {
    pub path: String,
    pub expected_hash: String,
}

/// A freshly generated proposal, pre-persistence (no id / status / timestamps).
#[derive(Debug, Clone)]
pub struct NewProposal {
    pub kind: ProposalKind,
    pub title: String,
    pub detail: String,
    pub action: ProposalAction,
    /// Stable dedup key so the same suggestion isn't re-queued every cycle.
    pub fingerprint: String,
}

/// A persisted proposal row (review queue).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceProposal {
    pub id: i64,
    pub kb_id: String,
    pub kind: ProposalKind,
    pub status: ProposalStatus,
    pub title: String,
    pub detail: String,
    pub action: ProposalAction,
    pub fingerprint: String,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Live status for the GUI (running flag + last cycle report).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceStatus {
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_report: Option<MaintenanceReport>,
}

/// Outcome of one maintenance cycle (generation pass).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceReport {
    /// Newly queued proposals this cycle.
    pub generated: usize,
    /// Count by kind (`as_str` key).
    pub by_kind: std::collections::BTreeMap<String, usize>,
    /// Candidates skipped because an identical proposal already exists.
    pub skipped_existing: usize,
    /// Of `generated`, how many were auto-applied (auto-approve on).
    pub auto_applied: usize,
    /// Reason when the cycle did nothing (disabled / already running / no KBs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub duration_ms: u64,
}
