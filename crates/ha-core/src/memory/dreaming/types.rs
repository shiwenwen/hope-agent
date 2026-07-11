//! Shared types for the Dreaming pipeline.

use serde::{Deserialize, Serialize};

use super::triggers::DreamTrigger;

/// A single provenance pointer for a promoted memory (Evidence Layer,
/// design §3.3). Phase 1 keeps evidence lightweight — it rides on the
/// fields the legacy `memories` row already carries (`source_session_id`
/// + the memory id), so no new schema is introduced. The dedicated
/// `memory_evidence` table with per-claim quotes lands with the claim
/// schema in a later PR.
///
/// `message_id` is reserved: the extraction path operates on API-shaped
/// history (role + content only, no DB ids), so precise message-level
/// anchors arrive once claim extraction has the LLM cite them per claim.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceRef {
    /// "memory" | "session_message".
    pub source_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Reserved for precise message anchors (filled by claim extraction).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<i64>,
}

impl EvidenceRef {
    /// Trace to the promoted memory itself — always available.
    pub fn memory(memory_id: i64) -> Self {
        Self {
            source_type: "memory".to_string(),
            memory_id: Some(memory_id),
            session_id: None,
            message_id: None,
        }
    }

    /// Trace to the originating session. Only built for non-incognito
    /// sources (see `scanner::evidence_for_candidate`).
    pub fn session(session_id: impl Into<String>) -> Self {
        Self {
            source_type: "session_message".to_string(),
            memory_id: None,
            session_id: Some(session_id.into()),
            message_id: None,
        }
    }
}

/// Summary of a single promotion decision.
/// Emitted back to the UI / diary; also written into the Dream Diary
/// markdown as a `<!-- ha-dream-promotion: ... -->` comment so the file
/// is both human-readable and machine-indexable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromotionRecord {
    /// ID of the promoted `MemoryEntry` in the backend.
    pub memory_id: i64,
    /// Score the LLM assigned (0.0–1.0), post-filter.
    pub score: f32,
    /// One-sentence title / headline for this memory (derived by the LLM).
    pub title: String,
    /// Short human-readable rationale for why it was promoted.
    pub rationale: String,
    /// Provenance pointers for this promotion (Evidence Layer). Empty when
    /// the source candidate could not be matched. Defaults to empty on
    /// deserialize so older serialized reports stay readable.
    #[serde(default)]
    pub evidence: Vec<EvidenceRef>,
}

/// Phase of a dreaming run. `Light` = Promotion (rule + one side_query);
/// `Deep` = the resolver (expire / merge / conflict); `Profile` = Memory
/// Profile synthesis (Phase 4, rule-based on idle/cron, LLM rewrite on
/// manual). Persisted as the lowercase string into `dreaming_runs.phase` /
/// lock keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DreamPhase {
    Light,
    Deep,
    Profile,
}

impl DreamPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            DreamPhase::Light => "light",
            DreamPhase::Deep => "deep",
            DreamPhase::Profile => "profile",
        }
    }
}

/// Lifecycle status of a durable `dreaming_runs` row.
///
/// `Running` is the transient state while a cycle is in flight; a crash
/// leaves it `Running` until startup recovery flips it to `Failed`. Pre-run
/// gating (disabled / lock contention) never creates a row, so `Skipped` is
/// reserved for future use and not written in Phase 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DreamRunStatus {
    Running,
    Completed,
    Failed,
    Skipped,
}

impl DreamRunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            DreamRunStatus::Running => "running",
            DreamRunStatus::Completed => "completed",
            DreamRunStatus::Failed => "failed",
            DreamRunStatus::Skipped => "skipped",
        }
    }
}

/// Terminal outcome of a dreaming cycle. Serialised into the trigger
/// response payload and logged.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamReport {
    /// Id of the durable `dreaming_runs` row for this cycle. `None` when the
    /// cycle was skipped before a run row was created (feature disabled,
    /// manual gating, or lock contention) or when the durable store is
    /// unavailable. Lets the Dashboard correlate the report to `get_run`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Which trigger fired this cycle.
    pub trigger: DreamTrigger,
    /// Total candidates scanned from the memory backend.
    pub candidates_scanned: usize,
    /// Candidates that the LLM nominated for promotion (pre-filter).
    pub candidates_nominated: usize,
    /// Candidates actually pinned after applying `min_score` and
    /// `max_promote` cutoffs.
    pub promoted: Vec<PromotionRecord>,
    /// Absolute path to the written Dream Diary markdown file (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diary_path: Option<String>,
    /// Total wall-clock duration (ms).
    pub duration_ms: u64,
    /// Human-readable notes (e.g. "no active agent", "backend empty").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A durable run row from `dreaming_runs`, returned by `dreaming_list_runs`
/// / `dreaming_get_run`. Survives restart (unlike the in-process
/// `last_report_snapshot`), so the Dashboard can show real run history.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamingRunRecord {
    pub id: String,
    /// "idle" | "cron" | "manual".
    pub trigger: String,
    /// "light" | "deep" | "profile".
    pub phase: String,
    /// "running" | "completed" | "failed" | "skipped".
    pub status: String,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    pub duration_ms: u64,
    pub candidates_scanned: usize,
    pub candidates_nominated: usize,
    pub promoted_count: usize,
    pub decision_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diary_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A single decision row from `dreaming_decisions` (Phase 0 writes only
/// `promote`). Returned inside `DreamingRunDetail`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamingDecisionRecord {
    pub id: String,
    /// promote | merge | supersede | expire | archive | needs_review | no_op.
    pub decision_type: String,
    /// memory | claim | profile | task | skill_suggestion.
    pub target_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    pub rationale: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_json: Option<String>,
    pub created_at: String,
}

/// Owner-plane query filter for durable decision history. Used by Review
/// Inbox audit search; this is read-only and never exposed as an agent tool.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamingDecisionListFilter {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub query: Option<String>,
    pub decision_type: Option<String>,
    pub scope_type: Option<String>,
    pub scope_id: Option<String>,
    pub since: Option<String>,
    pub target_type: Option<String>,
}

/// A decision row plus the run metadata needed for the Review Inbox audit
/// list. `content` / `scope*` are best-effort parsed from before/after JSON so
/// callers do not need to understand the raw snapshots for common filtering.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamingDecisionListItem {
    pub id: String,
    pub run_id: String,
    pub decision_type: String,
    pub target_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    pub rationale: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_json: Option<String>,
    pub created_at: String,
    pub run_trigger: String,
    pub run_phase: String,
    pub run_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
}

/// A page of decision rows with the total number of rows matching the same
/// filter. `total_truncated` is true when the scope/content post-filter hit
/// the bounded scan guard before proving an exact total.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamingDecisionListResponse {
    pub items: Vec<DreamingDecisionListItem>,
    pub total: usize,
    pub total_truncated: bool,
}

/// A run plus its decision log — returned by `dreaming_get_run`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamingRunDetail {
    pub run: DreamingRunRecord,
    pub decisions: Vec<DreamingDecisionRecord>,
}

/// Authorized, redacted excerpt of an evidence source — returned by
/// `dreaming_evidence_quote`. The backend resolves the quote so expansion
/// can never bypass the incognito gate by hiding it only in the frontend
/// (design §8.1 / §5.3: "Evidence 展开经过后端授权").
///
/// When the source can't be surfaced (`available = false`), `quote` is
/// empty and `reason` explains why (`incognito` / `not_found` /
/// `no_session_db` / `load_failed`); the backend never leaks incognito
/// content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceQuote {
    pub session_id: String,
    /// The message the quote came from (resolved server-side; `None` when
    /// unavailable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<i64>,
    /// Role of the quoted message ("user" / "assistant" / ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Redacted + length-capped excerpt. Empty when `available = false`.
    pub quote: String,
    /// Whether the excerpt was truncated to the char cap.
    pub truncated: bool,
    /// Whether a quote could be surfaced at all.
    pub available: bool,
    /// Machine-readable reason when `available = false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A persisted Memory Profile snapshot row from `memory_profile_snapshots`
/// (next-gen Dreaming Phase 4). Returned by the owner-plane read API for the
/// Settings / Dashboard read-only profile view. Read-only — users drill from
/// a profile line down to the claim, but editing lands with the correction
/// loop in a later PR.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSnapshotRecord {
    /// "global" | "agent" | "project".
    pub scope_type: String,
    /// Agent / project id; `None` for global (stored as `''` in the DB).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    /// Monotonic version (latest = highest), allocated `MAX(version)+1`.
    pub version: i64,
    /// Rendered profile markdown body (bullets / short prose).
    pub body_md: String,
    /// Optional claim provenance for profile bullets. Old snapshots and backup
    /// restores may not have this; the UI must degrade to plain Markdown.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<ProfileSnapshotSourceRecord>,
    /// The `dreaming_runs.id` that produced this snapshot (provenance).
    pub source_run_id: String,
    pub created_at: String,
}

/// Claim provenance for one profile snapshot bullet. `line_index` is 0-based
/// over rendered profile body lines after trimming empty lines; `None` means
/// scope-level provenance when exact line mapping is not available.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSnapshotSourceRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_index: Option<usize>,
    pub claim_id: String,
    pub claim_type: String,
    pub content: String,
    pub confidence: f32,
    pub salience: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_source_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_quote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_url: Option<String>,
}
