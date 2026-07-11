use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::memory::claims::ClaimDetail;
use crate::memory::dreaming::ProfileSnapshotRecord;
use crate::memory::episodes::{
    MemoryEpisodeRecord, MemoryExperienceHistoryRecord, MemoryProcedureRecord,
};

const MAX_EXTERNAL_MEMORY_PROVIDERS: usize = 16;
const MAX_EXTERNAL_PROVIDER_ID_CHARS: usize = 64;
const MAX_EXTERNAL_PROVIDER_DISPLAY_CHARS: usize = 80;
const MAX_EXTERNAL_PROVIDER_ERROR_CHARS: usize = 512;
const MAX_EXTERNAL_PROVIDER_TIMESTAMP_CHARS: usize = 80;

// ── Data Structures ─────────────────────────────────────────────

/// Memory entry types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    /// Information about the end user
    User,
    /// User feedback and preferences about agent behavior
    Feedback,
    /// Project-specific context and knowledge
    Project,
    /// Reference materials and external resource pointers
    Reference,
}

impl MemoryType {
    pub fn as_str(&self) -> &str {
        match self {
            MemoryType::User => "user",
            MemoryType::Feedback => "feedback",
            MemoryType::Project => "project",
            MemoryType::Reference => "reference",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "feedback" => MemoryType::Feedback,
            "project" => MemoryType::Project,
            "reference" => MemoryType::Reference,
            _ => MemoryType::User,
        }
    }

    /// Display heading for system prompt summary.
    pub(crate) fn heading(&self) -> &str {
        match self {
            MemoryType::User => "About the User",
            MemoryType::Feedback => "Preferences & Feedback",
            MemoryType::Project => "Project Context",
            MemoryType::Reference => "References",
        }
    }
}

/// Memory scope: global (shared across agents), per-agent (private to an
/// agent), or per-project (shared across all sessions inside a project).
///
/// Project scope is narrower than Agent scope — it exists so that when a
/// session belongs to a project, the project's knowledge takes precedence
/// and does not leak into other projects even if they use the same agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum MemoryScope {
    Global,
    Agent { id: String },
    Project { id: String },
}

/// A stored memory entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntry {
    pub id: i64,
    pub memory_type: MemoryType,
    pub scope: MemoryScope,
    pub content: String,
    pub tags: Vec<String>,
    /// Source: "user" (manual), "auto" (agent-extracted), "import"
    pub source: String,
    pub source_session_id: Option<String>,
    /// Whether this memory is pinned (always prioritized in system prompt injection)
    #[serde(default)]
    pub pinned: bool,
    pub created_at: String,
    pub updated_at: String,
    /// Populated during search, not stored in DB
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relevance_score: Option<f32>,
    /// Absolute path to attached file (image/audio), stored in ~/.hope-agent/memory_attachments/
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_path: Option<String>,
    /// MIME type of the attachment (e.g. "image/jpeg", "audio/mpeg")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_mime: Option<String>,
}

/// Durable owner-visible audit action for legacy memory rows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryHistoryAction {
    Add,
    Update,
    Delete,
    Pin,
    Unpin,
    Import,
}

impl MemoryHistoryAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryHistoryAction::Add => "add",
            MemoryHistoryAction::Update => "update",
            MemoryHistoryAction::Delete => "delete",
            MemoryHistoryAction::Pin => "pin",
            MemoryHistoryAction::Unpin => "unpin",
            MemoryHistoryAction::Import => "import",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "update" => MemoryHistoryAction::Update,
            "delete" => MemoryHistoryAction::Delete,
            "pin" => MemoryHistoryAction::Pin,
            "unpin" => MemoryHistoryAction::Unpin,
            "import" => MemoryHistoryAction::Import,
            _ => MemoryHistoryAction::Add,
        }
    }
}

/// Append-only audit row for ordinary legacy memory operations. It stores a
/// bounded preview, not a second full source of truth.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryHistoryRecord {
    pub id: String,
    pub memory_id: i64,
    pub action: MemoryHistoryAction,
    pub memory_type: MemoryType,
    pub scope: MemoryScope,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<String>,
    pub content_preview: String,
    #[serde(default)]
    pub pinned: bool,
    pub created_at: String,
}

/// Owner-plane query for durable legacy-memory audit rows. This is intentionally
/// separate from agent-facing recall/search so audit exploration cannot become
/// another prompt-injection path.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryHistoryQuery {
    /// Case-insensitive substring match over preview/source/session metadata.
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub actions: Option<Vec<MemoryHistoryAction>>,
    #[serde(default)]
    pub memory_types: Option<Vec<MemoryType>>,
    #[serde(default)]
    pub sources: Option<Vec<String>>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
}

/// Page response for owner-plane legacy-memory audit browsing. `total` is the
/// number of rows matching the same filters when the backend can count them
/// exactly; `total_truncated` lets minimal backends report a bounded estimate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryHistoryListResponse {
    pub items: Vec<MemoryHistoryRecord>,
    pub total: usize,
    #[serde(default)]
    pub total_truncated: bool,
}

/// Input for creating a new memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewMemory {
    pub memory_type: MemoryType,
    pub scope: MemoryScope,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default)]
    pub source_session_id: Option<String>,
    /// Whether this memory should be pinned (prioritized in system prompt)
    #[serde(default)]
    pub pinned: bool,
    /// Absolute path to an image/audio file attachment
    #[serde(default)]
    pub attachment_path: Option<String>,
    /// MIME type of the attachment
    #[serde(default)]
    pub attachment_mime: Option<String>,
}

fn default_source() -> String {
    "user".to_string()
}

/// Search query parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySearchQuery {
    /// Natural language query text
    pub query: String,
    /// Filter by type(s)
    #[serde(default)]
    pub types: Option<Vec<MemoryType>>,
    /// Filter by source(s): user/manual, auto, auto-reflect, auto-claim, import, etc.
    #[serde(default)]
    pub sources: Option<Vec<String>>,
    /// Filter by scope
    #[serde(default)]
    pub scope: Option<MemoryScope>,
    /// Shorthand: load global + this agent's memories
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Max results (default 20)
    #[serde(default)]
    pub limit: Option<usize>,
}

// ── Statistics ──────────────────────────────────────────────────

/// Memory statistics for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStats {
    pub total: usize,
    pub by_type: std::collections::HashMap<String, usize>,
    pub by_source: std::collections::HashMap<String, usize>,
    pub with_embedding: usize,
    pub oldest: Option<String>,
    pub newest: Option<String>,
}

/// Overall health state for the memory backend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryHealthStatus {
    Ok,
    Warning,
    Error,
}

/// Severity of a single memory health finding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryHealthSeverity {
    Info,
    Warning,
    Error,
}

/// One actionable health finding. `code` is stable for UI mapping and tests;
/// `message` / `action` are concise fallbacks for logs and API clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryHealthIssue {
    pub code: String,
    pub severity: MemoryHealthSeverity,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

/// Latest raw SQLite safety snapshot discovered by the owner health check.
/// It is informational and does not imply the snapshot has been restored.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDbSnapshotArtifact {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default)]
    pub status: MemoryDbSnapshotStatus,
    #[serde(default)]
    pub issues: Vec<String>,
    #[serde(default)]
    pub files: Vec<MemoryRepairArtifactFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryDbSnapshotRestoreStatus {
    #[default]
    Ready,
    NoMetadata,
    MissingFiles,
    SizeMismatch,
    Sha256Mismatch,
    QuickCheckFailed,
}

impl MemoryDbSnapshotRestoreStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryDbSnapshotRestoreStatus::Ready => "ready",
            MemoryDbSnapshotRestoreStatus::NoMetadata => "no_metadata",
            MemoryDbSnapshotRestoreStatus::MissingFiles => "missing_files",
            MemoryDbSnapshotRestoreStatus::SizeMismatch => "size_mismatch",
            MemoryDbSnapshotRestoreStatus::Sha256Mismatch => "sha256_mismatch",
            MemoryDbSnapshotRestoreStatus::QuickCheckFailed => "quick_check_failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryDbSnapshotFileStatus {
    #[default]
    Ok,
    Unverified,
    Missing,
    SizeMismatch,
    Sha256Mismatch,
}

/// Read-only restore preflight for a raw SQLite DB safety snapshot. It never
/// replaces the active database; owner UI must show this plan before any future
/// destructive restore / replace action.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDbSnapshotRestorePreview {
    pub snapshot_path: String,
    pub current_db_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    pub status: MemoryDbSnapshotRestoreStatus,
    pub can_restore: bool,
    pub quick_check: String,
    #[serde(default)]
    pub issues: Vec<String>,
    #[serde(default)]
    pub files: Vec<MemoryDbSnapshotRestoreFileCheck>,
}

/// Result of an explicit owner-triggered SQLite DB restore from a verified
/// safety snapshot. The active DB is restored through SQLite's backup API,
/// never by renaming files under live connections.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDbSnapshotRestoreReport {
    pub restored: bool,
    pub snapshot_path: String,
    pub rollback_snapshot_path: String,
    #[serde(default)]
    pub rollback_snapshot_files: Vec<MemoryRepairArtifactFile>,
    pub preflight: MemoryDbSnapshotRestorePreview,
    pub before: MemoryHealth,
    pub after: MemoryHealth,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDbSnapshotRestoreFileCheck {
    pub name: String,
    pub snapshot_path: String,
    pub target_path: String,
    pub status: MemoryDbSnapshotFileStatus,
    pub expected_size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_size_bytes: Option<u64>,
    pub expected_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryDbSnapshotStatus {
    #[default]
    Ok,
    NoMetadata,
    MissingFiles,
    SizeMismatch,
}

impl MemoryDbSnapshotStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryDbSnapshotStatus::Ok => "ok",
            MemoryDbSnapshotStatus::NoMetadata => "no_metadata",
            MemoryDbSnapshotStatus::MissingFiles => "missing_files",
            MemoryDbSnapshotStatus::SizeMismatch => "size_mismatch",
        }
    }
}

/// Read-only diagnostics for the long-term memory store. This intentionally
/// does not repair anything; owner surfaces can present the suggested action
/// and call existing reembed / export / maintenance commands explicitly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryHealth {
    pub backend_kind: String,
    pub status: MemoryHealthStatus,
    pub checked_at: String,
    pub quick_check: String,
    pub total_memories: usize,
    pub memories_with_active_embedding: usize,
    pub memories_pending_embedding: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_embedding_signature: Option<String>,
    #[serde(default)]
    pub embedding_provider_configured: bool,
    #[serde(default)]
    pub embedding_provider_loaded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_provider_dimensions: Option<u32>,
    #[serde(default)]
    pub embedding_provider_multimodal: bool,
    #[serde(default)]
    pub embedding_provider_batch: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector_rows: Option<usize>,
    pub fts_rows: usize,
    pub fts_missing_rows: usize,
    pub claims_total: usize,
    pub claims_needs_review: usize,
    pub claims_without_evidence: usize,
    #[serde(default)]
    pub claim_fts_rows: usize,
    #[serde(default)]
    pub claim_fts_missing_rows: usize,
    #[serde(default)]
    pub evidence_fts_rows: usize,
    #[serde(default)]
    pub evidence_fts_missing_rows: usize,
    pub orphan_evidence_rows: usize,
    pub orphan_claim_links: usize,
    #[serde(default)]
    pub episodes_total: usize,
    #[serde(default)]
    pub procedures_total: usize,
    #[serde(default)]
    pub orphan_procedure_episode_refs: usize,
    #[serde(default)]
    pub dreaming_running_runs: usize,
    #[serde(default)]
    pub dreaming_stale_runs: usize,
    #[serde(default)]
    pub dreaming_locks: usize,
    #[serde(default)]
    pub dreaming_stale_locks: usize,
    #[serde(default)]
    pub deep_resolver_active_claims: usize,
    #[serde(default)]
    pub deep_resolver_expired_candidates: usize,
    #[serde(default)]
    pub deep_resolver_conflict_groups: usize,
    #[serde(default)]
    pub deep_resolver_groups_to_analyze: usize,
    #[serde(default)]
    pub deep_resolver_group_cap: usize,
    #[serde(default)]
    pub deep_resolver_truncated: bool,
    #[serde(default)]
    pub deep_resolver_would_call_llm: bool,
    #[serde(default)]
    pub deep_resolver_blocking_reasons: Vec<String>,
    #[serde(default)]
    pub external_providers_enabled: bool,
    #[serde(default)]
    pub external_provider_count: usize,
    #[serde(default)]
    pub external_provider_active_count: usize,
    #[serde(default)]
    pub external_providers: Vec<ExternalMemoryProviderHealth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_db_snapshot: Option<MemoryDbSnapshotArtifact>,
    pub issues: Vec<MemoryHealthIssue>,
}

/// Supported external memory provider families. These are additive provider
/// adapters, not replacements for the local SQLite truth source.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExternalMemoryProviderKind {
    Mem0,
    Zep,
    Supermemory,
    Honcho,
    Hindsight,
    OpenViking,
    Custom,
}

impl ExternalMemoryProviderKind {
    pub const ALL: [Self; 7] = [
        Self::Mem0,
        Self::Zep,
        Self::Supermemory,
        Self::Honcho,
        Self::Hindsight,
        Self::OpenViking,
        Self::Custom,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mem0 => "mem0",
            Self::Zep => "zep",
            Self::Supermemory => "supermemory",
            Self::Honcho => "honcho",
            Self::Hindsight => "hindsight",
            Self::OpenViking => "open_viking",
            Self::Custom => "custom",
        }
    }

    pub fn capabilities(&self) -> ExternalMemoryProviderCapabilities {
        // Keep this registry explicit: health, preflight and owner sync all
        // project runtime readiness from this one table.
        match self {
            Self::Mem0
            | Self::Zep
            | Self::Supermemory
            | Self::Honcho
            | Self::Hindsight
            | Self::OpenViking
            | Self::Custom => ExternalMemoryProviderCapabilities {
                adapter_available: true,
                requires_endpoint: true,
                supports_manual: true,
                supports_pull: true,
                supports_push: true,
                supports_bidirectional: true,
            },
        }
    }
}

/// External provider sync policy. `Off` is the default and the only policy
/// that has no external IO. Active policies remain explicit opt-ins and do not
/// change local prompt / recall semantics.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExternalMemorySyncPolicy {
    #[default]
    Off,
    Manual,
    PullOnly,
    PushOnly,
    Bidirectional,
}

impl ExternalMemorySyncPolicy {
    pub fn is_active(&self) -> bool {
        !matches!(self, Self::Off)
    }

    pub fn data_flow(&self) -> ExternalMemoryProviderDataFlow {
        match self {
            Self::Off => ExternalMemoryProviderDataFlow::None,
            Self::Manual => ExternalMemoryProviderDataFlow::Manual,
            Self::PullOnly => ExternalMemoryProviderDataFlow::PullOnly,
            Self::PushOnly => ExternalMemoryProviderDataFlow::PushOnly,
            Self::Bidirectional => ExternalMemoryProviderDataFlow::Bidirectional,
        }
    }

    pub fn sends_query_context(&self) -> bool {
        matches!(self, Self::Manual | Self::PullOnly | Self::Bidirectional)
    }

    pub fn sends_local_memory(&self) -> bool {
        matches!(self, Self::Manual | Self::PushOnly | Self::Bidirectional)
    }

    pub fn imports_external_memory(&self) -> bool {
        matches!(self, Self::Manual | Self::PullOnly | Self::Bidirectional)
    }

    pub fn supported_by(&self, capabilities: &ExternalMemoryProviderCapabilities) -> bool {
        match self {
            Self::Off => true,
            Self::Manual => capabilities.supports_manual,
            Self::PullOnly => capabilities.supports_pull,
            Self::PushOnly => capabilities.supports_push,
            Self::Bidirectional => capabilities.supports_bidirectional,
        }
    }
}

/// Owner-visible data-flow projection for external provider policies. This is
/// not an execution log; it separates user-selected policy intent from runtime
/// adapter readiness so health consumers do not confuse planned sync with data
/// actually being able to move.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExternalMemoryProviderDataFlow {
    #[default]
    None,
    Manual,
    PullOnly,
    PushOnly,
    Bidirectional,
}

/// Runtime capability contract for an external provider family. This is health
/// metadata, not user data and not a credential record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalMemoryProviderCapabilities {
    #[serde(default)]
    pub adapter_available: bool,
    #[serde(default)]
    pub requires_endpoint: bool,
    #[serde(default)]
    pub supports_manual: bool,
    #[serde(default)]
    pub supports_pull: bool,
    #[serde(default)]
    pub supports_push: bool,
    #[serde(default)]
    pub supports_bidirectional: bool,
}

impl Default for ExternalMemoryProviderCapabilities {
    fn default() -> Self {
        Self {
            adapter_available: false,
            requires_endpoint: true,
            supports_manual: false,
            supports_pull: false,
            supports_push: false,
            supports_bidirectional: false,
        }
    }
}

/// Stable owner-facing reasons why an external provider cannot currently sync.
/// These are diagnostics only; they do not perform or authorize any network IO.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExternalMemoryProviderSyncBlockReason {
    GlobalDisabled,
    ProviderDisabled,
    PolicyOff,
    EndpointMissing,
    PolicyUnsupported,
    AdapterUnavailable,
    LastError,
}

/// Persisted, non-secret provider config. Secrets must stay in the platform
/// credential store or environment references when concrete adapters land.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalMemoryProviderConfig {
    pub id: String,
    pub kind: ExternalMemoryProviderKind,
    pub display_name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub sync_policy: ExternalMemorySyncPolicy,
    #[serde(default)]
    pub endpoint_configured: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// Global external memory provider switch. Default disabled so no user memory
/// leaves the device unless a future owner UI explicitly opts in.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalMemoryProvidersConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub providers: Vec<ExternalMemoryProviderConfig>,
}

impl ExternalMemoryProvidersConfig {
    pub fn normalized(mut self) -> Self {
        self.providers.truncate(MAX_EXTERNAL_MEMORY_PROVIDERS);
        let mut used_ids = HashSet::new();
        self.providers = self
            .providers
            .into_iter()
            .enumerate()
            .map(|(index, provider)| normalize_external_provider(provider, index, &mut used_ids))
            .collect();
        self
    }
}

/// Owner-visible health projection for one external provider config.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalMemoryProviderHealth {
    pub id: String,
    pub kind: ExternalMemoryProviderKind,
    pub display_name: String,
    pub enabled: bool,
    pub sync_policy: ExternalMemorySyncPolicy,
    pub status: MemoryHealthStatus,
    #[serde(default)]
    pub capabilities: ExternalMemoryProviderCapabilities,
    #[serde(default = "default_true")]
    pub policy_supported: bool,
    #[serde(default)]
    pub policy_data_flow: ExternalMemoryProviderDataFlow,
    #[serde(default)]
    pub runtime_data_flow: ExternalMemoryProviderDataFlow,
    #[serde(default)]
    pub runtime_sync_enabled: bool,
    #[serde(default)]
    pub sync_blocked: bool,
    #[serde(default)]
    pub sync_block_reasons: Vec<ExternalMemoryProviderSyncBlockReason>,
    #[serde(default)]
    pub sends_query_context: bool,
    #[serde(default)]
    pub sends_local_memory: bool,
    #[serde(default)]
    pub imports_external_memory: bool,
    #[serde(default)]
    pub requires_explicit_action: bool,
    #[serde(default)]
    pub automatic_sync: bool,
    #[serde(default)]
    pub endpoint_configured: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// Owner-only dry-run action state for external memory sync. This is a
/// preflight projection, not an execution result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExternalMemoryProviderPreflightAction {
    Off,
    Blocked,
    WouldSync,
}

/// Owner-visible result status for a requested external provider sync run.
/// This is distinct from preflight so future real adapters can report executed
/// success/failure without changing the dry-run action contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExternalMemoryProviderSyncStatus {
    Off,
    Blocked,
    NoRuntimeAdapter,
    Succeeded,
    Failed,
}

/// One provider's owner-visible external sync preflight. It intentionally
/// exposes counts and data-flow flags, never memory contents or credentials.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalMemoryProviderPreflight {
    pub id: String,
    pub kind: ExternalMemoryProviderKind,
    pub display_name: String,
    pub action: ExternalMemoryProviderPreflightAction,
    pub dry_run_only: bool,
    pub health: ExternalMemoryProviderHealth,
    pub planned_data_flow: ExternalMemoryProviderDataFlow,
    pub runtime_data_flow: ExternalMemoryProviderDataFlow,
    pub planned_sends_query_context: bool,
    pub planned_sends_local_memory: bool,
    pub planned_imports_external_memory: bool,
    pub runtime_sends_query_context: bool,
    pub runtime_sends_local_memory: bool,
    pub runtime_imports_external_memory: bool,
    pub local_memory_candidate_count: usize,
}

/// One provider's owner-visible sync result. Counts are zero until a concrete
/// adapter executes; blocked/no-adapter results must not imply any IO happened.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalMemoryProviderSyncResult {
    pub id: String,
    pub kind: ExternalMemoryProviderKind,
    pub display_name: String,
    pub status: ExternalMemoryProviderSyncStatus,
    pub external_io_performed: bool,
    pub preflight: ExternalMemoryProviderPreflight,
    pub imported_memory_count: usize,
    pub exported_memory_count: usize,
    pub updated_memory_count: usize,
    pub skipped_memory_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Owner-only report for an external memory provider sync request. Current
/// planned adapters return blocked/off results only; future real adapters must
/// keep local SQLite as truth source and fill result counts after explicit IO.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalMemoryProviderSyncReport {
    pub generated_at: String,
    pub global_enabled: bool,
    pub external_io_performed: bool,
    pub local_memory_total: usize,
    pub local_memory_with_embedding: usize,
    #[serde(default)]
    pub stats_unavailable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats_error: Option<String>,
    pub runnable_provider_count: usize,
    pub blocked_provider_count: usize,
    pub executed_provider_count: usize,
    pub succeeded_provider_count: usize,
    pub failed_provider_count: usize,
    pub providers: Vec<ExternalMemoryProviderSyncResult>,
}

/// Owner-only dry-run report for external memory sync. Future concrete
/// adapters should keep this as the explicit preflight before any outbound IO.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalMemoryProviderPreflightReport {
    pub generated_at: String,
    pub global_enabled: bool,
    pub dry_run_only: bool,
    pub local_memory_total: usize,
    pub local_memory_with_embedding: usize,
    #[serde(default)]
    pub stats_unavailable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats_error: Option<String>,
    pub runnable_provider_count: usize,
    pub blocked_provider_count: usize,
    pub providers: Vec<ExternalMemoryProviderPreflight>,
}

impl ExternalMemoryProviderHealth {
    pub fn from_config(config: &ExternalMemoryProviderConfig, global_enabled: bool) -> Self {
        Self::from_config_with_capabilities(config, global_enabled, config.kind.capabilities())
    }

    pub fn from_config_with_capabilities(
        config: &ExternalMemoryProviderConfig,
        global_enabled: bool,
        capabilities: ExternalMemoryProviderCapabilities,
    ) -> Self {
        let active = global_enabled && config.enabled && config.sync_policy.is_active();
        let policy_supported = config.sync_policy.supported_by(&capabilities);
        let endpoint_ready = !capabilities.requires_endpoint || config.endpoint_configured;
        let runtime_sync_enabled =
            active && policy_supported && capabilities.adapter_available && endpoint_ready;
        let sync_block_reasons = external_provider_sync_block_reasons(
            config,
            global_enabled,
            &capabilities,
            policy_supported,
            endpoint_ready,
        );
        let policy_data_flow = if active {
            config.sync_policy.data_flow()
        } else {
            ExternalMemoryProviderDataFlow::None
        };
        let runtime_data_flow = if runtime_sync_enabled {
            policy_data_flow.clone()
        } else {
            ExternalMemoryProviderDataFlow::None
        };
        let status = if !active {
            MemoryHealthStatus::Ok
        } else if !policy_supported
            || !capabilities.adapter_available
            || !endpoint_ready
            || config.last_error.is_some()
        {
            MemoryHealthStatus::Warning
        } else {
            MemoryHealthStatus::Ok
        };
        Self {
            id: config.id.clone(),
            kind: config.kind.clone(),
            display_name: config.display_name.clone(),
            enabled: active,
            sync_policy: config.sync_policy.clone(),
            status,
            capabilities,
            policy_supported,
            policy_data_flow,
            runtime_data_flow,
            runtime_sync_enabled,
            sync_blocked: active && !runtime_sync_enabled,
            sync_block_reasons,
            sends_query_context: runtime_sync_enabled && config.sync_policy.sends_query_context(),
            sends_local_memory: runtime_sync_enabled && config.sync_policy.sends_local_memory(),
            imports_external_memory: runtime_sync_enabled
                && config.sync_policy.imports_external_memory(),
            requires_explicit_action: runtime_sync_enabled
                && matches!(config.sync_policy, ExternalMemorySyncPolicy::Manual),
            automatic_sync: runtime_sync_enabled
                && matches!(
                    config.sync_policy,
                    ExternalMemorySyncPolicy::PullOnly
                        | ExternalMemorySyncPolicy::PushOnly
                        | ExternalMemorySyncPolicy::Bidirectional
                ),
            endpoint_configured: config.endpoint_configured,
            last_sync_at: config.last_sync_at.clone(),
            last_error: config.last_error.clone(),
        }
    }
}

impl ExternalMemoryProvidersConfig {
    pub fn sync_preflight(&self, stats: &MemoryStats) -> ExternalMemoryProviderPreflightReport {
        self.sync_preflight_with_stats_status(stats, None)
    }

    pub fn sync_preflight_with_stats_status(
        &self,
        stats: &MemoryStats,
        stats_error: Option<String>,
    ) -> ExternalMemoryProviderPreflightReport {
        let providers = self
            .providers
            .iter()
            .map(|provider| {
                let health = ExternalMemoryProviderHealth::from_config(provider, self.enabled);
                external_memory_provider_preflight(provider, health, self.enabled, stats)
            })
            .collect::<Vec<_>>();
        let runnable_provider_count = providers
            .iter()
            .filter(|provider| provider.action == ExternalMemoryProviderPreflightAction::WouldSync)
            .count();
        let blocked_provider_count = providers
            .iter()
            .filter(|provider| provider.action == ExternalMemoryProviderPreflightAction::Blocked)
            .count();
        ExternalMemoryProviderPreflightReport {
            generated_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            global_enabled: self.enabled,
            dry_run_only: true,
            local_memory_total: stats.total,
            local_memory_with_embedding: stats.with_embedding,
            stats_unavailable: stats_error.is_some(),
            stats_error,
            runnable_provider_count,
            blocked_provider_count,
            providers,
        }
    }

    pub fn sync_report(&self, stats: &MemoryStats) -> ExternalMemoryProviderSyncReport {
        self.sync_report_with_stats_status(stats, None)
    }

    pub fn sync_report_with_stats_status(
        &self,
        stats: &MemoryStats,
        stats_error: Option<String>,
    ) -> ExternalMemoryProviderSyncReport {
        let preflight = self.sync_preflight_with_stats_status(stats, stats_error);
        external_memory_provider_sync_report_from_preflight(preflight)
    }
}

fn external_memory_provider_sync_report_from_preflight(
    preflight: ExternalMemoryProviderPreflightReport,
) -> ExternalMemoryProviderSyncReport {
    let providers = preflight
        .providers
        .into_iter()
        .map(external_memory_provider_sync_result_from_preflight)
        .collect::<Vec<_>>();
    let external_io_performed = providers
        .iter()
        .any(|provider| provider.external_io_performed);
    let executed_provider_count = providers
        .iter()
        .filter(|provider| provider.external_io_performed)
        .count();
    let succeeded_provider_count = providers
        .iter()
        .filter(|provider| provider.status == ExternalMemoryProviderSyncStatus::Succeeded)
        .count();
    let failed_provider_count = providers
        .iter()
        .filter(|provider| provider.status == ExternalMemoryProviderSyncStatus::Failed)
        .count();
    let blocked_provider_count = providers
        .iter()
        .filter(|provider| {
            matches!(
                provider.status,
                ExternalMemoryProviderSyncStatus::Blocked
                    | ExternalMemoryProviderSyncStatus::NoRuntimeAdapter
            )
        })
        .count();

    ExternalMemoryProviderSyncReport {
        generated_at: preflight.generated_at,
        global_enabled: preflight.global_enabled,
        external_io_performed,
        local_memory_total: preflight.local_memory_total,
        local_memory_with_embedding: preflight.local_memory_with_embedding,
        stats_unavailable: preflight.stats_unavailable,
        stats_error: preflight.stats_error,
        runnable_provider_count: preflight.runnable_provider_count,
        blocked_provider_count,
        executed_provider_count,
        succeeded_provider_count,
        failed_provider_count,
        providers,
    }
}

fn external_memory_provider_sync_result_from_preflight(
    preflight: ExternalMemoryProviderPreflight,
) -> ExternalMemoryProviderSyncResult {
    let status = match &preflight.action {
        ExternalMemoryProviderPreflightAction::Off => ExternalMemoryProviderSyncStatus::Off,
        ExternalMemoryProviderPreflightAction::Blocked => ExternalMemoryProviderSyncStatus::Blocked,
        ExternalMemoryProviderPreflightAction::WouldSync => {
            // Fail closed until a concrete provider adapter owns this branch.
            ExternalMemoryProviderSyncStatus::NoRuntimeAdapter
        }
    };
    let error = if status == ExternalMemoryProviderSyncStatus::NoRuntimeAdapter {
        Some("external memory provider runtime adapter is not wired".to_string())
    } else {
        None
    };
    ExternalMemoryProviderSyncResult {
        id: preflight.id.clone(),
        kind: preflight.kind.clone(),
        display_name: preflight.display_name.clone(),
        status,
        external_io_performed: false,
        preflight,
        imported_memory_count: 0,
        exported_memory_count: 0,
        updated_memory_count: 0,
        skipped_memory_count: 0,
        error,
    }
}

fn external_memory_provider_preflight(
    provider: &ExternalMemoryProviderConfig,
    health: ExternalMemoryProviderHealth,
    global_enabled: bool,
    stats: &MemoryStats,
) -> ExternalMemoryProviderPreflight {
    let active = global_enabled && provider.enabled && provider.sync_policy.is_active();
    let action = if !active {
        ExternalMemoryProviderPreflightAction::Off
    } else if health.runtime_sync_enabled {
        ExternalMemoryProviderPreflightAction::WouldSync
    } else {
        ExternalMemoryProviderPreflightAction::Blocked
    };
    let planned_sends_query_context = active && provider.sync_policy.sends_query_context();
    let planned_sends_local_memory = active && provider.sync_policy.sends_local_memory();
    let planned_imports_external_memory = active && provider.sync_policy.imports_external_memory();
    ExternalMemoryProviderPreflight {
        id: provider.id.clone(),
        kind: provider.kind.clone(),
        display_name: provider.display_name.clone(),
        action,
        dry_run_only: true,
        planned_data_flow: if active {
            provider.sync_policy.data_flow()
        } else {
            ExternalMemoryProviderDataFlow::None
        },
        runtime_data_flow: health.runtime_data_flow.clone(),
        planned_sends_query_context,
        planned_sends_local_memory,
        planned_imports_external_memory,
        runtime_sends_query_context: health.sends_query_context,
        runtime_sends_local_memory: health.sends_local_memory,
        runtime_imports_external_memory: health.imports_external_memory,
        local_memory_candidate_count: if planned_sends_local_memory {
            stats.total
        } else {
            0
        },
        health,
    }
}

fn external_provider_sync_block_reasons(
    config: &ExternalMemoryProviderConfig,
    global_enabled: bool,
    capabilities: &ExternalMemoryProviderCapabilities,
    policy_supported: bool,
    endpoint_ready: bool,
) -> Vec<ExternalMemoryProviderSyncBlockReason> {
    let mut reasons = Vec::new();
    if !global_enabled {
        reasons.push(ExternalMemoryProviderSyncBlockReason::GlobalDisabled);
    }
    if !config.enabled {
        reasons.push(ExternalMemoryProviderSyncBlockReason::ProviderDisabled);
    }
    if !config.sync_policy.is_active() {
        reasons.push(ExternalMemoryProviderSyncBlockReason::PolicyOff);
    }
    if config.sync_policy.is_active() {
        if !endpoint_ready {
            reasons.push(ExternalMemoryProviderSyncBlockReason::EndpointMissing);
        }
        if !policy_supported {
            reasons.push(ExternalMemoryProviderSyncBlockReason::PolicyUnsupported);
        }
        if !capabilities.adapter_available {
            reasons.push(ExternalMemoryProviderSyncBlockReason::AdapterUnavailable);
        }
        if config.last_error.is_some() {
            reasons.push(ExternalMemoryProviderSyncBlockReason::LastError);
        }
    }
    reasons
}

fn normalize_external_provider(
    mut provider: ExternalMemoryProviderConfig,
    index: usize,
    used_ids: &mut HashSet<String>,
) -> ExternalMemoryProviderConfig {
    let fallback_id = format!("{}-{}", provider.kind.as_str(), index + 1);
    let base_id = sanitize_external_provider_id(&provider.id)
        .filter(|id| !id.is_empty())
        .unwrap_or(fallback_id);
    provider.id = unique_external_provider_id(base_id, used_ids);

    provider.display_name = truncate_chars(
        provider.display_name.trim(),
        MAX_EXTERNAL_PROVIDER_DISPLAY_CHARS,
    );
    if provider.display_name.is_empty() {
        provider.display_name = provider.kind.as_str().to_string();
    }

    provider.last_error = provider
        .last_error
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| truncate_chars(value, MAX_EXTERNAL_PROVIDER_ERROR_CHARS));
    provider.last_sync_at = provider
        .last_sync_at
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| truncate_chars(value, MAX_EXTERNAL_PROVIDER_TIMESTAMP_CHARS));
    provider
}

fn sanitize_external_provider_id(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut last_was_sep = false;
    for ch in raw.trim().chars().flat_map(char::to_lowercase) {
        let mapped = if ch.is_ascii_alphanumeric() {
            Some(ch)
        } else if matches!(ch, '-' | '_' | ' ' | '.' | '/') {
            Some('-')
        } else {
            None
        };
        let Some(ch) = mapped else {
            continue;
        };
        if ch == '-' {
            if last_was_sep {
                continue;
            }
            last_was_sep = true;
        } else {
            last_was_sep = false;
        }
        out.push(ch);
        if out.len() >= MAX_EXTERNAL_PROVIDER_ID_CHARS {
            break;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn unique_external_provider_id(base: String, used_ids: &mut HashSet<String>) -> String {
    if used_ids.insert(base.clone()) {
        return base;
    }
    let prefix = base
        .chars()
        .take(MAX_EXTERNAL_PROVIDER_ID_CHARS.saturating_sub(4))
        .collect::<String>();
    for suffix in 2..=999usize {
        let candidate = format!("{prefix}-{suffix}");
        if used_ids.insert(candidate.clone()) {
            return candidate;
        }
    }
    let fallback = format!("provider-{}", used_ids.len() + 1);
    used_ids.insert(fallback.clone());
    fallback
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

/// Owner-only repair actions for rebuildable memory indexes. Each action must
/// be conservative: no source-of-truth memory / claim content is rewritten.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryRepairAction {
    RebuildFts,
    RebuildClaimFts,
    RepairClaimGraph,
    RepairExperienceGraph,
    RecoverDreamingState,
    CreateDbSnapshot,
}

/// File copied into a repair artifact, with enough metadata for a future
/// restore flow or support workflow to verify the snapshot before use.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRepairArtifactFile {
    pub name: String,
    pub size_bytes: u64,
    pub sha256: String,
}

/// Result of a repair action, including before/after diagnostics so owner UIs
/// can show exactly what changed without running an extra round-trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRepairReport {
    pub action: MemoryRepairAction,
    pub changed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_files: Vec<MemoryRepairArtifactFile>,
    pub before: MemoryHealth,
    pub after: MemoryHealth,
}

/// Summary and integrity notes for a memory backup bundle. `complete=false`
/// means at least one optional section could not be read; the exported JSON is
/// still useful, but owner UIs should surface `warnings`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBackupManifest {
    pub complete: bool,
    pub legacy_memory_count: usize,
    #[serde(default)]
    pub legacy_history_count: usize,
    pub attachment_ref_count: usize,
    #[serde(default)]
    pub attachment_payload_count: usize,
    #[serde(default)]
    pub attachment_chunk_count: usize,
    #[serde(default)]
    pub attachment_chunked_ref_count: usize,
    #[serde(default)]
    pub attachment_external_ref_count: usize,
    #[serde(default)]
    pub attachment_payload_bytes: u64,
    #[serde(default)]
    pub attachment_missing_count: usize,
    pub claim_count: usize,
    pub evidence_count: usize,
    pub claim_link_count: usize,
    pub profile_snapshot_count: usize,
    #[serde(default)]
    pub episode_count: usize,
    #[serde(default)]
    pub procedure_count: usize,
    #[serde(default)]
    pub experience_history_count: usize,
    pub unsupported_sections: Vec<String>,
    pub warnings: Vec<String>,
}

/// Inline payload for a long-term memory attachment. Payloads are keyed to the
/// source bundle's legacy memory id; restore writes them into this machine's
/// `memory_attachments` directory and rewrites the restored memory path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBackupAttachmentPayload {
    pub memory_id: i64,
    pub original_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    pub size_bytes: u64,
    pub sha256: String,
    pub base64: String,
}

/// One chunk of a long-term memory attachment. Chunked payloads let backup
/// bundles carry larger files without requiring a single huge base64 string.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBackupAttachmentChunkPayload {
    pub memory_id: i64,
    pub original_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    pub size_bytes: u64,
    pub sha256: String,
    pub chunk_index: usize,
    pub chunk_count: usize,
    pub chunk_size_bytes: u64,
    pub chunk_sha256: String,
    pub base64: String,
}

/// Metadata for an attachment that is too large for the JSON backup payload.
/// The binary is not embedded yet; sidecar packagers/importers can use this to
/// match and verify an external payload without trusting a local path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBackupAttachmentExternalPayload {
    pub memory_id: i64,
    pub original_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    pub size_bytes: u64,
    pub sha256: String,
    pub sidecar_file_name: String,
    pub reason: String,
}

/// Portable, read-only snapshot of the local memory system. This is intentionally
/// export-only for now: restore/import will go through a separate preview plan
/// so a malformed or stale bundle can never overwrite local memory silently.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBackupBundle {
    pub schema_version: String,
    pub exported_at: String,
    pub app_version: String,
    pub manifest: MemoryBackupManifest,
    pub stats: MemoryStats,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<MemoryHealth>,
    pub config_manifest: serde_json::Value,
    pub legacy_memories: Vec<MemoryEntry>,
    #[serde(default)]
    pub legacy_history: Vec<MemoryHistoryRecord>,
    #[serde(default)]
    pub attachment_payloads: Vec<MemoryBackupAttachmentPayload>,
    #[serde(default)]
    pub attachment_payload_chunks: Vec<MemoryBackupAttachmentChunkPayload>,
    #[serde(default)]
    pub attachment_external_payloads: Vec<MemoryBackupAttachmentExternalPayload>,
    pub legacy_markdown: String,
    pub claims: Vec<ClaimDetail>,
    pub profile_snapshots: Vec<ProfileSnapshotRecord>,
    #[serde(default)]
    pub episodes: Vec<MemoryEpisodeRecord>,
    #[serde(default)]
    pub procedures: Vec<MemoryProcedureRecord>,
    #[serde(default)]
    pub experience_history: Vec<MemoryExperienceHistoryRecord>,
}

/// Password-encrypted wrapper around a `MemoryBackupBundle`. The encrypted
/// payload decrypts back to the normal bundle schema so every preview/restore
/// path can stay single-source and fail-closed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEncryptedBackupBundle {
    pub schema_version: String,
    pub exported_at: String,
    pub app_version: String,
    pub plaintext_schema_version: String,
    pub kdf: MemoryBackupKdfManifest,
    pub cipher: MemoryBackupCipherManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBackupKdfManifest {
    pub name: String,
    pub iterations: u32,
    pub salt_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBackupCipherManifest {
    pub name: String,
    pub nonce_base64: String,
    pub ciphertext_base64: String,
    pub mac_base64: String,
}

/// One issue discovered while previewing a backup bundle import.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBackupPreviewIssue {
    pub severity: MemoryHealthSeverity,
    pub code: String,
    pub message: String,
}

/// A compact, user-facing example of an incoming backup claim that conflicts
/// with an existing local claim. This is preview guidance only; restore still
/// routes conflicts into Review Inbox instead of overwriting local memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBackupClaimConflictExample {
    pub incoming_claim_id: String,
    pub existing_claim_id: String,
    pub scope: String,
    pub claim_type: String,
    pub subject: String,
    pub predicate: String,
    pub incoming_object: String,
    pub existing_object: String,
    pub incoming_content: String,
    pub existing_content: String,
}

/// Read-only restore plan for structured claims. Apply paths consume the same
/// plan shape so owner UIs can explain what will be restored or reviewed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBackupClaimRestorePlan {
    pub total: usize,
    pub existing_by_id: usize,
    pub exact_matches: usize,
    pub import_candidates: usize,
    #[serde(default)]
    pub conflicting_candidates: usize,
    pub needs_review_candidates: usize,
    pub archived_candidates: usize,
    pub superseded_candidates: usize,
    pub expired_candidates: usize,
    pub manual_evidence_rows: usize,
    pub by_type: BTreeMap<String, usize>,
    pub by_status: BTreeMap<String, usize>,
    #[serde(default)]
    pub conflict_examples: Vec<MemoryBackupClaimConflictExample>,
    pub preview_only: bool,
}

impl Default for MemoryBackupClaimRestorePlan {
    fn default() -> Self {
        Self {
            total: 0,
            existing_by_id: 0,
            exact_matches: 0,
            import_candidates: 0,
            conflicting_candidates: 0,
            needs_review_candidates: 0,
            archived_candidates: 0,
            superseded_candidates: 0,
            expired_candidates: 0,
            manual_evidence_rows: 0,
            by_type: BTreeMap::new(),
            by_status: BTreeMap::new(),
            conflict_examples: Vec::new(),
            preview_only: true,
        }
    }
}

/// Read-only restore plan for Memory Profile snapshots. Restore execution will
/// need a separate merge/apply path because snapshots are versioned per scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBackupProfileRestorePlan {
    pub total: usize,
    pub matching_scopes: usize,
    pub exact_matches: usize,
    pub import_candidates: usize,
    pub conflicting_scope_candidates: usize,
    pub by_scope_type: BTreeMap<String, usize>,
    pub preview_only: bool,
}

impl Default for MemoryBackupProfileRestorePlan {
    fn default() -> Self {
        Self {
            total: 0,
            matching_scopes: 0,
            exact_matches: 0,
            import_candidates: 0,
            conflicting_scope_candidates: 0,
            by_scope_type: BTreeMap::new(),
            preview_only: true,
        }
    }
}

/// Read-only import preview for a backup bundle. It is deliberately separate
/// from import execution so restore flows can remain reviewable and fail-closed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBackupImportPreview {
    pub valid: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exported_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_manifest: Option<MemoryBackupManifest>,
    pub current_stats: MemoryStats,
    pub legacy_memory_count: usize,
    pub legacy_exact_matches: usize,
    pub legacy_import_candidates: usize,
    pub legacy_duplicate_in_bundle: usize,
    #[serde(default)]
    pub legacy_history_count: usize,
    #[serde(default)]
    pub legacy_history_restorable: usize,
    #[serde(default)]
    pub legacy_history_skipped_unmapped: usize,
    pub attachment_ref_count: usize,
    #[serde(default)]
    pub attachment_payload_count: usize,
    #[serde(default)]
    pub attachment_chunk_count: usize,
    #[serde(default)]
    pub attachment_chunked_ref_count: usize,
    #[serde(default)]
    pub attachment_external_ref_count: usize,
    #[serde(default)]
    pub attachment_external_available_count: usize,
    #[serde(default)]
    pub attachment_payload_bytes: u64,
    #[serde(default)]
    pub attachment_missing_count: usize,
    pub claim_count: usize,
    pub claim_id_matches: usize,
    #[serde(default)]
    pub claim_restore_plan: MemoryBackupClaimRestorePlan,
    pub evidence_count: usize,
    pub claim_link_count: usize,
    pub profile_snapshot_count: usize,
    #[serde(default)]
    pub profile_restore_plan: MemoryBackupProfileRestorePlan,
    #[serde(default)]
    pub episode_count: usize,
    #[serde(default)]
    pub episode_id_matches: usize,
    #[serde(default)]
    pub episode_exact_matches: usize,
    #[serde(default)]
    pub episode_import_candidates: usize,
    #[serde(default)]
    pub procedure_count: usize,
    #[serde(default)]
    pub procedure_id_matches: usize,
    #[serde(default)]
    pub procedure_exact_matches: usize,
    #[serde(default)]
    pub procedure_import_candidates: usize,
    #[serde(default)]
    pub experience_history_count: usize,
    #[serde(default)]
    pub experience_history_restorable: usize,
    #[serde(default)]
    pub experience_history_skipped_unmapped: usize,
    pub unsupported_sections: Vec<String>,
    pub issues: Vec<MemoryBackupPreviewIssue>,
    pub next_steps: Vec<String>,
}

/// Options for applying the legacy-memory subset of a backup bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBackupRestoreOptions {
    #[serde(default = "default_backup_restore_dedup")]
    pub dedup: bool,
}

fn default_backup_restore_dedup() -> bool {
    true
}

impl Default for MemoryBackupRestoreOptions {
    fn default() -> Self {
        Self {
            dedup: default_backup_restore_dedup(),
        }
    }
}

/// Options for applying the structured subset of a backup bundle. Defaults are
/// conservative: import missing claims and profile snapshots, but do not make a
/// conflicting profile snapshot the latest profile for an already-populated
/// scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBackupStructuredRestoreOptions {
    #[serde(default = "default_true")]
    pub restore_claims: bool,
    #[serde(default = "default_true")]
    pub restore_profile_snapshots: bool,
    #[serde(default = "default_true")]
    pub restore_episodes: bool,
    #[serde(default = "default_true")]
    pub restore_procedures: bool,
    #[serde(default = "default_true")]
    pub restore_experience_history: bool,
    #[serde(default)]
    pub allow_profile_scope_conflicts: bool,
}

fn default_true() -> bool {
    true
}

impl Default for MemoryBackupStructuredRestoreOptions {
    fn default() -> Self {
        Self {
            restore_claims: true,
            restore_profile_snapshots: true,
            restore_episodes: true,
            restore_procedures: true,
            restore_experience_history: true,
            allow_profile_scope_conflicts: false,
        }
    }
}

/// Result of applying the safe subset of a memory backup bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBackupRestoreResult {
    pub preview: MemoryBackupImportPreview,
    pub import_result: ImportResult,
    pub attempted_legacy_memories: usize,
    pub skipped_exact_matches: usize,
    pub skipped_duplicate_in_bundle: usize,
    pub skipped_attachment_refs: usize,
    pub restored_attachments: usize,
    #[serde(default)]
    pub restored_legacy_history: usize,
    #[serde(default)]
    pub skipped_legacy_history_unmapped: usize,
    pub preview_only_claims: usize,
    pub preview_only_profile_snapshots: usize,
}

/// Result of applying the structured subset of a memory backup bundle.
/// Legacy memory rows are not written here; callers can run legacy restore
/// first, then structured restore will map claim links by exact memory
/// fingerprint where possible.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBackupStructuredRestoreResult {
    pub preview: MemoryBackupImportPreview,
    pub restored_claims: usize,
    pub restored_claims_needing_review: usize,
    pub skipped_claim_id_matches: usize,
    pub skipped_claim_exact_matches: usize,
    pub restored_evidence_rows: usize,
    pub restored_claim_links: usize,
    pub skipped_claim_links: usize,
    pub failed_claims: usize,
    pub restored_profile_snapshots: usize,
    pub skipped_profile_exact_matches: usize,
    pub skipped_profile_scope_conflicts: usize,
    pub failed_profile_snapshots: usize,
    #[serde(default)]
    pub restored_episodes: usize,
    #[serde(default)]
    pub skipped_episode_id_matches: usize,
    #[serde(default)]
    pub skipped_episode_exact_matches: usize,
    #[serde(default)]
    pub failed_episodes: usize,
    #[serde(default)]
    pub restored_procedures: usize,
    #[serde(default)]
    pub skipped_procedure_id_matches: usize,
    #[serde(default)]
    pub skipped_procedure_exact_matches: usize,
    #[serde(default)]
    pub failed_procedures: usize,
    #[serde(default)]
    pub restored_experience_history: usize,
    #[serde(default)]
    pub skipped_experience_history_unmapped: usize,
    pub errors: Vec<String>,
}

impl MemoryBackupImportPreview {
    pub fn empty(current_stats: MemoryStats) -> Self {
        Self {
            valid: false,
            schema_version: None,
            exported_at: None,
            app_version: None,
            source_manifest: None,
            current_stats,
            legacy_memory_count: 0,
            legacy_exact_matches: 0,
            legacy_import_candidates: 0,
            legacy_duplicate_in_bundle: 0,
            legacy_history_count: 0,
            legacy_history_restorable: 0,
            legacy_history_skipped_unmapped: 0,
            attachment_ref_count: 0,
            attachment_payload_count: 0,
            attachment_chunk_count: 0,
            attachment_chunked_ref_count: 0,
            attachment_external_ref_count: 0,
            attachment_external_available_count: 0,
            attachment_payload_bytes: 0,
            attachment_missing_count: 0,
            claim_count: 0,
            claim_id_matches: 0,
            claim_restore_plan: MemoryBackupClaimRestorePlan::default(),
            evidence_count: 0,
            claim_link_count: 0,
            profile_snapshot_count: 0,
            profile_restore_plan: MemoryBackupProfileRestorePlan::default(),
            episode_count: 0,
            episode_id_matches: 0,
            episode_exact_matches: 0,
            episode_import_candidates: 0,
            procedure_count: 0,
            procedure_id_matches: 0,
            procedure_exact_matches: 0,
            procedure_import_candidates: 0,
            experience_history_count: 0,
            experience_history_restorable: 0,
            experience_history_skipped_unmapped: 0,
            unsupported_sections: Vec::new(),
            issues: Vec::new(),
            next_steps: Vec::new(),
        }
    }
}

impl MemoryHealth {
    pub fn new(backend_kind: impl Into<String>, stats: &MemoryStats) -> Self {
        Self {
            backend_kind: backend_kind.into(),
            status: MemoryHealthStatus::Ok,
            checked_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            quick_check: "not_checked".to_string(),
            total_memories: stats.total,
            memories_with_active_embedding: stats.with_embedding,
            memories_pending_embedding: 0,
            active_embedding_signature: None,
            embedding_provider_configured: false,
            embedding_provider_loaded: false,
            embedding_provider_dimensions: None,
            embedding_provider_multimodal: false,
            embedding_provider_batch: false,
            vector_rows: None,
            fts_rows: 0,
            fts_missing_rows: 0,
            claims_total: 0,
            claims_needs_review: 0,
            claims_without_evidence: 0,
            claim_fts_rows: 0,
            claim_fts_missing_rows: 0,
            evidence_fts_rows: 0,
            evidence_fts_missing_rows: 0,
            orphan_evidence_rows: 0,
            orphan_claim_links: 0,
            episodes_total: 0,
            procedures_total: 0,
            orphan_procedure_episode_refs: 0,
            dreaming_running_runs: 0,
            dreaming_stale_runs: 0,
            dreaming_locks: 0,
            dreaming_stale_locks: 0,
            deep_resolver_active_claims: 0,
            deep_resolver_expired_candidates: 0,
            deep_resolver_conflict_groups: 0,
            deep_resolver_groups_to_analyze: 0,
            deep_resolver_group_cap: 0,
            deep_resolver_truncated: false,
            deep_resolver_would_call_llm: false,
            deep_resolver_blocking_reasons: Vec::new(),
            external_providers_enabled: false,
            external_provider_count: 0,
            external_provider_active_count: 0,
            external_providers: Vec::new(),
            latest_db_snapshot: None,
            issues: Vec::new(),
        }
    }

    pub fn add_issue(
        &mut self,
        code: impl Into<String>,
        severity: MemoryHealthSeverity,
        message: impl Into<String>,
        action: Option<String>,
    ) {
        self.issues.push(MemoryHealthIssue {
            code: code.into(),
            severity,
            message: message.into(),
            action,
        });
        self.refresh_status();
    }

    pub fn refresh_status(&mut self) {
        self.status = if self
            .issues
            .iter()
            .any(|issue| issue.severity == MemoryHealthSeverity::Error)
        {
            MemoryHealthStatus::Error
        } else if self
            .issues
            .iter()
            .any(|issue| issue.severity == MemoryHealthSeverity::Warning)
        {
            MemoryHealthStatus::Warning
        } else {
            MemoryHealthStatus::Ok
        };
    }
}

// ── Global Memory Extract Config ────────────────────────────────

/// Global auto-extract configuration, stored in config.json `memoryExtract` field.
/// Per-agent MemoryConfig can override these with Some(...) values.
///
/// Trigger logic (since last extraction):
/// - Cooldown: elapsed time must >= `extract_time_threshold_secs` (prevents too-frequent extraction)
/// - Trigger: token count >= `extract_token_threshold` OR message count >= `extract_message_threshold`
/// Both cooldown AND trigger must be satisfied.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryExtractConfig {
    /// Global long-term memory runtime switch. When false, the agent plane must
    /// not inject, recall, auto-extract, flush, or write persistent memory.
    /// Owner-plane management APIs remain available so users can inspect,
    /// export, delete, or re-enable their existing data.
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
    #[serde(default = "crate::default_true")]
    pub auto_extract: bool,
    /// Deprecated — superseded by `modelOverride`. Kept for backward
    /// compatibility: still read when `modelOverride` is unset, but the GUI
    /// no longer writes these two fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_model_id: Option<String>,
    /// Model override for auto-extraction. `None` = fall through to the
    /// deprecated `extractProviderId`/`extractModelId` pair (if both set) →
    /// the current session's own model. Per-agent `memory.extractProviderId`/
    /// `extractModelId` (`AgentModelConfig`) still takes precedence over all
    /// of this when set — unchanged, not part of this reshape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<crate::provider::ActiveModel>,
    /// Auto-extract memories before context compaction (Tier 3 summarization)
    #[serde(default = "crate::default_true")]
    pub flush_before_compact: bool,
    /// Token accumulation threshold — trigger extraction when tokens since last extraction >= this (default: 8000)
    #[serde(default = "default_extract_token_threshold")]
    pub extract_token_threshold: usize,
    /// Cooldown in seconds — extraction won't trigger until this much time has passed (default: 300 = 5 min)
    #[serde(default = "default_extract_time_threshold_secs")]
    pub extract_time_threshold_secs: u64,
    /// Message count threshold — trigger extraction when messages since last extraction >= this (default: 10)
    #[serde(default = "default_extract_message_threshold")]
    pub extract_message_threshold: usize,
    /// Idle timeout in seconds — trigger final extraction when session is idle for this long (default: 1800 = 30 min). 0 = disabled.
    #[serde(default = "default_extract_idle_timeout_secs")]
    pub extract_idle_timeout_secs: u64,
    /// Phase B'2 — enable reflective extraction alongside factual extraction.
    /// When true, each auto-extract pass asks the model to surface user
    /// profile traits (communication style, work habits) and tags them
    /// `profile` so they render in the `## User Profile` system-prompt section.
    /// Default: true. Runs in the same side_query roundtrip as `facts`.
    #[serde(default = "crate::default_true")]
    pub enable_reflection: bool,
    /// Next-gen Dreaming (beta) — also extract structured `memory_claims` +
    /// `memory_evidence` alongside the legacy `facts`/`profile`, and dual-write
    /// each claim's shadow into `memories` (linked via `memory_claim_links`).
    /// Runs in the SAME side_query roundtrip as `facts` (a third `claims`
    /// array). Default ON — the structured claim layer ships enabled so the
    /// out-of-box experience builds claims / profile without manual opt-in;
    /// also gates the Dashboard "Claims (beta)" view.
    #[serde(default = "crate::default_true")]
    pub extract_claims: bool,
    /// Review-first learning mode. When enabled, auto-extracted structured
    /// claims are written as `needs_review` first; their managed legacy shadows
    /// stay hidden until the user approves the claim. Manual `save_memory`,
    /// claim corrections, backfill, and restore flows keep their own policies.
    #[serde(default)]
    pub review_first: bool,
}
fn default_extract_token_threshold() -> usize {
    8000
}
fn default_extract_time_threshold_secs() -> u64 {
    300
}
fn default_extract_message_threshold() -> usize {
    10
}
fn default_extract_idle_timeout_secs() -> u64 {
    1800
}

impl Default for MemoryExtractConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_extract: true,
            extract_provider_id: None,
            extract_model_id: None,
            model_override: None,
            flush_before_compact: true,
            extract_token_threshold: default_extract_token_threshold(),
            extract_time_threshold_secs: default_extract_time_threshold_secs(),
            extract_message_threshold: default_extract_message_threshold(),
            extract_idle_timeout_secs: default_extract_idle_timeout_secs(),
            enable_reflection: true,
            extract_claims: true,
            review_first: false,
        }
    }
}

// ── LLM Memory Selection ──────────────────────────────────────

/// LLM-based memory selection configuration, stored in config.json `memorySelection` field.
/// When enabled, uses side_query() to select the most relevant memories for the
/// current user message, reducing system prompt noise from irrelevant entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySelectionConfig {
    /// Enable LLM-based memory selection (default: false, opt-in)
    #[serde(default)]
    pub enabled: bool,
    /// Minimum candidate count before LLM selection kicks in (default: 8)
    #[serde(default = "default_selection_threshold")]
    pub threshold: usize,
    /// Maximum memories to select (default: 5)
    #[serde(default = "default_selection_max")]
    pub max_selected: usize,
}

fn default_selection_threshold() -> usize {
    8
}
fn default_selection_max() -> usize {
    5
}

impl Default for MemorySelectionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: 8,
            max_selected: 5,
        }
    }
}

// ── Memory Section Budget ───────────────────────────────────────

/// Per-section character budgets for the SQLite Layer 3 block of the
/// system-prompt memory section.
///
/// Default allocation is 15/20/20/30/15 = 10_000 chars total, matching the
/// default `MemoryBudgetConfig::total_chars`. `scaled_to` proportionally
/// shrinks the five sections when the caller-provided cap is smaller than
/// the sum (e.g., when Layer 1/2 consumed most of the total budget).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct SqliteSectionBudgets {
    /// Load-bearing: serde alias keeps pre-rename `config.json` / agent
    /// overrides (`"aboutYou": ...`) deserialising into this field. Do not
    /// remove until the back-compat window is officially closed.
    #[serde(alias = "aboutYou")]
    pub user_profile: usize,
    pub about_user: usize,
    pub preferences: usize,
    pub project_context: usize,
    pub references: usize,
}

impl Default for SqliteSectionBudgets {
    fn default() -> Self {
        Self {
            user_profile: 1500,
            about_user: 2000,
            preferences: 2000,
            project_context: 3000,
            references: 1500,
        }
    }
}

impl SqliteSectionBudgets {
    pub fn total(&self) -> usize {
        self.user_profile
            + self.about_user
            + self.preferences
            + self.project_context
            + self.references
    }

    /// Return a copy whose five sections fit inside `cap`, proportionally
    /// scaled from the configured values when they sum to more than `cap`.
    /// Returns a zeroed-out struct when `cap == 0`.
    pub fn scaled_to(&self, cap: usize) -> Self {
        let t = self.total();
        if t == 0 || cap == 0 {
            return Self {
                user_profile: 0,
                about_user: 0,
                preferences: 0,
                project_context: 0,
                references: 0,
            };
        }
        if t <= cap {
            return self.clone();
        }
        let ratio = cap as f64 / t as f64;
        Self {
            user_profile: (self.user_profile as f64 * ratio) as usize,
            about_user: (self.about_user as f64 * ratio) as usize,
            preferences: (self.preferences as f64 * ratio) as usize,
            project_context: (self.project_context as f64 * ratio) as usize,
            references: (self.references as f64 * ratio) as usize,
        }
    }
}

/// Controls how much of the system prompt the memory section is allowed to
/// consume. Defaults tuned for a ~10K char / ~2.5K token memory block that
/// hard-prioritises the Memory Guidelines epilogue and degrades gracefully
/// (Agent core memory > Global core memory > SQLite summary) when the total
/// budget is tighter than the individual per-layer caps.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryBudgetConfig {
    /// Hard upper bound on the entire memory section (Layer 1 + 2 + 3 + 4).
    pub total_chars: usize,
    /// Upper bound on each `memory.md` file (Global / Agent) individually.
    /// Actual injection is `min(core_memory_file_chars, remaining_total)`.
    pub core_memory_file_chars: usize,
    /// Per-entry truncation for rendered SQLite memory bullets (Layer 3).
    pub sqlite_entry_max_chars: usize,
    /// Per-section sub-budgets for the SQLite block.
    pub sqlite_sections: SqliteSectionBudgets,
}

impl Default for MemoryBudgetConfig {
    fn default() -> Self {
        Self {
            total_chars: 10_000,
            core_memory_file_chars: 8_000,
            sqlite_entry_max_chars: 500,
            sqlite_sections: SqliteSectionBudgets::default(),
        }
    }
}

// ── Deduplication ───────────────────────────────────────────────

/// Default dedup thresholds (RRF scores)
pub const DEDUP_THRESHOLD_HIGH: f32 = 0.02; // Above this → duplicate, skip
pub const DEDUP_THRESHOLD_MERGE: f32 = 0.012; // Between merge..high → update existing

/// Configurable dedup thresholds, stored in config.json `dedup` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DedupConfig {
    #[serde(default = "default_dedup_high")]
    pub threshold_high: f32,
    #[serde(default = "default_dedup_merge")]
    pub threshold_merge: f32,
}

fn default_dedup_high() -> f32 {
    DEDUP_THRESHOLD_HIGH
}
fn default_dedup_merge() -> f32 {
    DEDUP_THRESHOLD_MERGE
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self {
            threshold_high: DEDUP_THRESHOLD_HIGH,
            threshold_merge: DEDUP_THRESHOLD_MERGE,
        }
    }
}

/// Result of adding a memory with deduplication.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum AddResult {
    /// New memory created
    Created { id: i64 },
    /// Skipped — too similar to existing entry
    Duplicate { existing_id: i64, score: f32 },
    /// Updated existing entry with new content
    Updated { id: i64 },
}

// ── Hybrid Search Config ───────────────────────────────────────

/// Configurable hybrid search weights, stored in config.json `hybridSearch` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridSearchConfig {
    /// Weight for vector similarity results (0.0-1.0)
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f32,
    /// Weight for FTS keyword results (0.0-1.0)
    #[serde(default = "default_text_weight")]
    pub text_weight: f32,
    /// RRF constant k (higher = more equal weighting across ranks)
    #[serde(default = "default_rrf_k")]
    pub rrf_k: f64,
}

fn default_vector_weight() -> f32 {
    0.6
}
fn default_text_weight() -> f32 {
    0.4
}
fn default_rrf_k() -> f64 {
    60.0
}

impl Default for HybridSearchConfig {
    fn default() -> Self {
        Self {
            vector_weight: 0.6,
            text_weight: 0.4,
            rrf_k: 60.0,
        }
    }
}

// ── Temporal Decay Config ──────────────────────────────────────

/// Temporal decay configuration for memory search scoring.
/// Recent memories rank higher; pinned memories are exempt (evergreen).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemporalDecayConfig {
    /// Enable temporal decay (default: false)
    #[serde(default)]
    pub enabled: bool,
    /// Half-life in days: after this many days, score is halved (default: 30)
    #[serde(default = "default_half_life_days")]
    pub half_life_days: f64,
}

fn default_half_life_days() -> f64 {
    30.0
}

impl Default for TemporalDecayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            half_life_days: 30.0,
        }
    }
}

// ── MMR Config ─────────────────────────────────────────────────

/// MMR (Maximal Marginal Relevance) reranking config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MmrConfig {
    /// Enable MMR reranking (default: true)
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
    /// Lambda: 0 = max diversity, 1 = max relevance (default: 0.7)
    #[serde(default = "default_mmr_lambda")]
    pub lambda: f32,
}

fn default_mmr_lambda() -> f32 {
    0.7
}

impl Default for MmrConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            lambda: 0.7,
        }
    }
}

// ── Embedding Cache Config ─────────────────────────────────────

/// Configuration for caching computed embeddings to reduce API calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingCacheConfig {
    /// Enable embedding cache (default: true)
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
    /// Maximum number of cached entries (default: 10000)
    #[serde(default = "default_max_cache_entries")]
    pub max_entries: usize,
}

fn default_max_cache_entries() -> usize {
    10000
}

impl Default for EmbeddingCacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_entries: 10000,
        }
    }
}

// ── Multimodal Config ──────────────────────────────────────────

/// Supported modalities for multimodal embedding.
pub const MULTIMODAL_IMAGE_EXTENSIONS: &[(&str, &str)] = &[
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("png", "image/png"),
    ("webp", "image/webp"),
    ("gif", "image/gif"),
    ("heic", "image/heic"),
    ("heif", "image/heif"),
];

pub const MULTIMODAL_AUDIO_EXTENSIONS: &[(&str, &str)] = &[
    ("mp3", "audio/mpeg"),
    ("wav", "audio/wav"),
    ("ogg", "audio/ogg"),
    ("opus", "audio/opus"),
    ("m4a", "audio/mp4"),
    ("aac", "audio/aac"),
    ("flac", "audio/flac"),
];

/// Detect MIME type from file extension.
pub fn mime_from_extension(path: &str) -> Option<String> {
    let lower = path.to_lowercase();
    let ext = lower.rsplit('.').next()?;
    for (e, mime) in MULTIMODAL_IMAGE_EXTENSIONS
        .iter()
        .chain(MULTIMODAL_AUDIO_EXTENSIONS.iter())
    {
        if ext == *e {
            return Some(mime.to_string());
        }
    }
    None
}

/// Get modality label ("image" or "audio") from MIME type.
pub fn modality_label(mime: &str) -> &'static str {
    if mime.starts_with("image/") {
        "image"
    } else if mime.starts_with("audio/") {
        "audio"
    } else {
        "file"
    }
}

/// Multimodal embedding configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultimodalConfig {
    /// Enable multimodal embedding (default: false)
    #[serde(default)]
    pub enabled: bool,
    /// Supported modalities: "image", "audio"
    #[serde(default = "default_modalities")]
    pub modalities: Vec<String>,
    /// Max file size in bytes (default: 10MB)
    #[serde(default = "default_max_file_bytes")]
    pub max_file_bytes: u64,
}

fn default_modalities() -> Vec<String> {
    vec!["image".to_string(), "audio".to_string()]
}
fn default_max_file_bytes() -> u64 {
    10 * 1024 * 1024
} // 10MB

impl Default for MultimodalConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            modalities: default_modalities(),
            max_file_bytes: default_max_file_bytes(),
        }
    }
}

/// Result of a batch import operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub created: usize,
    pub skipped_duplicate: usize,
    pub failed: usize,
    pub errors: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_memory_providers_default_to_safe_off() {
        let cfg = ExternalMemoryProvidersConfig::default();

        assert!(!cfg.enabled);
        assert!(cfg.providers.is_empty());
    }

    #[test]
    fn external_memory_provider_kind_capabilities_cover_every_registered_adapter() {
        assert_eq!(ExternalMemoryProviderKind::ALL.len(), 7);

        for kind in ExternalMemoryProviderKind::ALL {
            let capabilities = kind.capabilities();
            assert!(
                capabilities.adapter_available,
                "all registered provider kinds must have a runtime adapter"
            );
            assert!(
                capabilities.requires_endpoint,
                "{} must require explicit endpoint setup",
                kind.as_str()
            );
            assert!(
                capabilities.supports_manual,
                "{} should expose the planned manual sync policy",
                kind.as_str()
            );
            assert!(
                capabilities.supports_pull,
                "{} should expose the planned pull policy",
                kind.as_str()
            );
            assert!(
                capabilities.supports_push,
                "{} should expose the planned push policy",
                kind.as_str()
            );
            assert!(
                capabilities.supports_bidirectional,
                "{} should expose the planned bidirectional policy",
                kind.as_str()
            );
        }
    }

    #[test]
    fn external_memory_provider_health_requires_global_enable_and_active_policy() {
        let provider = ExternalMemoryProviderConfig {
            id: "custom-main".to_string(),
            kind: ExternalMemoryProviderKind::Custom,
            display_name: "Custom".to_string(),
            enabled: true,
            sync_policy: ExternalMemorySyncPolicy::Off,
            endpoint_configured: true,
            last_sync_at: Some("2026-07-07T00:00:00Z".to_string()),
            last_error: Some("transient".to_string()),
        };

        let disabled = ExternalMemoryProviderHealth::from_config(&provider, false);
        assert!(!disabled.enabled);
        assert_eq!(disabled.status, MemoryHealthStatus::Ok);
        assert_eq!(
            disabled.sync_block_reasons,
            vec![
                ExternalMemoryProviderSyncBlockReason::GlobalDisabled,
                ExternalMemoryProviderSyncBlockReason::PolicyOff,
            ]
        );
        assert_eq!(
            disabled.policy_data_flow,
            ExternalMemoryProviderDataFlow::None
        );
        assert_eq!(
            disabled.runtime_data_flow,
            ExternalMemoryProviderDataFlow::None
        );
        assert!(!disabled.runtime_sync_enabled);

        let off_policy = ExternalMemoryProviderHealth::from_config(&provider, true);
        assert!(!off_policy.enabled);
        assert_eq!(off_policy.status, MemoryHealthStatus::Ok);
        assert_eq!(
            off_policy.sync_block_reasons,
            vec![ExternalMemoryProviderSyncBlockReason::PolicyOff]
        );
        assert_eq!(
            off_policy.policy_data_flow,
            ExternalMemoryProviderDataFlow::None
        );

        let active_provider = ExternalMemoryProviderConfig {
            sync_policy: ExternalMemorySyncPolicy::Manual,
            ..provider
        };
        let unavailable_capabilities = ExternalMemoryProviderCapabilities {
            adapter_available: false,
            requires_endpoint: true,
            supports_manual: true,
            supports_pull: true,
            supports_push: true,
            supports_bidirectional: true,
        };
        let active = ExternalMemoryProviderHealth::from_config_with_capabilities(
            &active_provider,
            true,
            unavailable_capabilities.clone(),
        );
        assert!(active.enabled);
        assert_eq!(active.status, MemoryHealthStatus::Warning);
        assert!(!active.capabilities.adapter_available);
        assert!(active.capabilities.requires_endpoint);
        assert_eq!(
            active.policy_data_flow,
            ExternalMemoryProviderDataFlow::Manual
        );
        assert_eq!(
            active.runtime_data_flow,
            ExternalMemoryProviderDataFlow::None
        );
        assert!(!active.runtime_sync_enabled);
        assert!(active.sync_blocked);
        assert!(!active.sends_query_context);
        assert!(!active.sends_local_memory);
        assert!(!active.imports_external_memory);
        assert_eq!(
            active.sync_block_reasons,
            vec![
                ExternalMemoryProviderSyncBlockReason::AdapterUnavailable,
                ExternalMemoryProviderSyncBlockReason::LastError,
            ]
        );

        let endpoint_ready_but_no_adapter = ExternalMemoryProviderConfig {
            last_error: None,
            endpoint_configured: true,
            ..active_provider.clone()
        };
        let adapter_pending = ExternalMemoryProviderHealth::from_config_with_capabilities(
            &endpoint_ready_but_no_adapter,
            true,
            unavailable_capabilities,
        );
        assert_eq!(adapter_pending.status, MemoryHealthStatus::Warning);
        assert!(!adapter_pending.capabilities.adapter_available);
        assert!(adapter_pending.policy_supported);
        assert_eq!(
            adapter_pending.policy_data_flow,
            ExternalMemoryProviderDataFlow::Manual
        );
        assert_eq!(
            adapter_pending.runtime_data_flow,
            ExternalMemoryProviderDataFlow::None
        );
        assert!(adapter_pending.sync_blocked);
        assert_eq!(
            adapter_pending.sync_block_reasons,
            vec![ExternalMemoryProviderSyncBlockReason::AdapterUnavailable]
        );

        let incomplete_provider = ExternalMemoryProviderConfig {
            sync_policy: ExternalMemorySyncPolicy::Manual,
            endpoint_configured: false,
            last_error: None,
            ..active_provider
        };
        let incomplete = ExternalMemoryProviderHealth::from_config(&incomplete_provider, true);
        assert!(incomplete.enabled);
        assert_eq!(incomplete.status, MemoryHealthStatus::Warning);
        assert!(incomplete
            .sync_block_reasons
            .contains(&ExternalMemoryProviderSyncBlockReason::EndpointMissing));
    }

    #[test]
    fn external_memory_provider_health_projects_runtime_data_flow_from_capabilities() {
        let provider = ExternalMemoryProviderConfig {
            id: "mem0-main".to_string(),
            kind: ExternalMemoryProviderKind::Mem0,
            display_name: "Mem0".to_string(),
            enabled: true,
            sync_policy: ExternalMemorySyncPolicy::PullOnly,
            endpoint_configured: true,
            last_sync_at: None,
            last_error: None,
        };
        let ready = ExternalMemoryProviderHealth::from_config_with_capabilities(
            &provider,
            true,
            ExternalMemoryProviderCapabilities {
                adapter_available: true,
                requires_endpoint: true,
                supports_manual: true,
                supports_pull: true,
                supports_push: true,
                supports_bidirectional: true,
            },
        );

        assert_eq!(ready.status, MemoryHealthStatus::Ok);
        assert_eq!(
            ready.policy_data_flow,
            ExternalMemoryProviderDataFlow::PullOnly
        );
        assert_eq!(
            ready.runtime_data_flow,
            ExternalMemoryProviderDataFlow::PullOnly
        );
        assert!(ready.runtime_sync_enabled);
        assert!(!ready.sync_blocked);
        assert!(ready.sends_query_context);
        assert!(!ready.sends_local_memory);
        assert!(ready.imports_external_memory);
        assert!(!ready.requires_explicit_action);
        assert!(ready.automatic_sync);
        assert!(ready.sync_block_reasons.is_empty());

        let push_only = ExternalMemoryProviderHealth::from_config_with_capabilities(
            &ExternalMemoryProviderConfig {
                sync_policy: ExternalMemorySyncPolicy::PushOnly,
                ..provider
            },
            true,
            ready.capabilities.clone(),
        );
        assert_eq!(
            push_only.runtime_data_flow,
            ExternalMemoryProviderDataFlow::PushOnly
        );
        assert!(!push_only.sends_query_context);
        assert!(push_only.sends_local_memory);
        assert!(!push_only.imports_external_memory);
        assert!(push_only.automatic_sync);
    }

    #[test]
    fn external_memory_provider_preflight_is_dry_run_and_does_not_claim_blocked_io() {
        let stats = MemoryStats {
            total: 42,
            by_type: std::collections::HashMap::new(),
            by_source: std::collections::HashMap::new(),
            with_embedding: 30,
            oldest: None,
            newest: None,
        };
        let cfg = ExternalMemoryProvidersConfig {
            enabled: true,
            providers: vec![ExternalMemoryProviderConfig {
                id: "zep-main".to_string(),
                kind: ExternalMemoryProviderKind::Zep,
                display_name: "Zep".to_string(),
                enabled: true,
                sync_policy: ExternalMemorySyncPolicy::Manual,
                endpoint_configured: false,
                last_sync_at: None,
                last_error: None,
            }],
        };

        let report = cfg.sync_preflight(&stats);
        assert!(report.dry_run_only);
        assert!(!report.stats_unavailable);
        assert_eq!(report.stats_error, None);
        assert_eq!(report.local_memory_total, 42);
        assert_eq!(report.local_memory_with_embedding, 30);
        assert_eq!(report.runnable_provider_count, 0);
        assert_eq!(report.blocked_provider_count, 1);
        let item = &report.providers[0];
        assert_eq!(item.action, ExternalMemoryProviderPreflightAction::Blocked);
        assert_eq!(
            item.planned_data_flow,
            ExternalMemoryProviderDataFlow::Manual
        );
        assert_eq!(item.runtime_data_flow, ExternalMemoryProviderDataFlow::None);
        assert!(item.planned_sends_query_context);
        assert!(item.planned_sends_local_memory);
        assert!(item.planned_imports_external_memory);
        assert!(!item.runtime_sends_query_context);
        assert!(!item.runtime_sends_local_memory);
        assert!(!item.runtime_imports_external_memory);
        assert_eq!(item.local_memory_candidate_count, 42);
        assert_eq!(
            item.health.sync_block_reasons,
            vec![ExternalMemoryProviderSyncBlockReason::EndpointMissing]
        );
    }

    #[test]
    fn external_memory_provider_sync_report_is_fail_closed_when_endpoint_is_missing() {
        let stats = MemoryStats {
            total: 42,
            by_type: std::collections::HashMap::new(),
            by_source: std::collections::HashMap::new(),
            with_embedding: 30,
            oldest: None,
            newest: None,
        };
        let cfg = ExternalMemoryProvidersConfig {
            enabled: true,
            providers: vec![ExternalMemoryProviderConfig {
                id: "zep-main".to_string(),
                kind: ExternalMemoryProviderKind::Zep,
                display_name: "Zep".to_string(),
                enabled: true,
                sync_policy: ExternalMemorySyncPolicy::Manual,
                endpoint_configured: false,
                last_sync_at: None,
                last_error: None,
            }],
        };

        let report = cfg.sync_report(&stats);

        assert!(!report.external_io_performed);
        assert_eq!(report.local_memory_total, 42);
        assert_eq!(report.local_memory_with_embedding, 30);
        assert_eq!(report.runnable_provider_count, 0);
        assert_eq!(report.blocked_provider_count, 1);
        assert_eq!(report.executed_provider_count, 0);
        assert_eq!(report.succeeded_provider_count, 0);
        assert_eq!(report.failed_provider_count, 0);
        let result = &report.providers[0];
        assert_eq!(result.status, ExternalMemoryProviderSyncStatus::Blocked);
        assert!(!result.external_io_performed);
        assert_eq!(
            result.preflight.action,
            ExternalMemoryProviderPreflightAction::Blocked
        );
        assert_eq!(result.imported_memory_count, 0);
        assert_eq!(result.exported_memory_count, 0);
        assert_eq!(result.updated_memory_count, 0);
        assert_eq!(result.skipped_memory_count, 0);
        assert_eq!(result.error, None);
        assert_eq!(
            result.preflight.health.sync_block_reasons,
            vec![ExternalMemoryProviderSyncBlockReason::EndpointMissing]
        );
    }

    #[test]
    fn external_memory_provider_sync_report_blocks_ready_preflight_without_executor() {
        let provider = ExternalMemoryProviderConfig {
            id: "custom-main".to_string(),
            kind: ExternalMemoryProviderKind::Custom,
            display_name: "Custom".to_string(),
            enabled: true,
            sync_policy: ExternalMemorySyncPolicy::PushOnly,
            endpoint_configured: true,
            last_sync_at: None,
            last_error: None,
        };
        let health = ExternalMemoryProviderHealth::from_config_with_capabilities(
            &provider,
            true,
            ExternalMemoryProviderCapabilities {
                adapter_available: true,
                requires_endpoint: true,
                supports_manual: true,
                supports_pull: true,
                supports_push: true,
                supports_bidirectional: false,
            },
        );
        let preflight = ExternalMemoryProviderPreflightReport {
            generated_at: "2026-07-08T00:00:00.000Z".to_string(),
            global_enabled: true,
            dry_run_only: true,
            local_memory_total: 7,
            local_memory_with_embedding: 2,
            stats_unavailable: false,
            stats_error: None,
            runnable_provider_count: 1,
            blocked_provider_count: 0,
            providers: vec![external_memory_provider_preflight(
                &provider,
                health,
                true,
                &MemoryStats {
                    total: 7,
                    by_type: std::collections::HashMap::new(),
                    by_source: std::collections::HashMap::new(),
                    with_embedding: 2,
                    oldest: None,
                    newest: None,
                },
            )],
        };

        let report = external_memory_provider_sync_report_from_preflight(preflight);

        assert!(!report.external_io_performed);
        assert_eq!(report.runnable_provider_count, 1);
        assert_eq!(report.blocked_provider_count, 1);
        assert_eq!(report.executed_provider_count, 0);
        let result = &report.providers[0];
        assert_eq!(
            result.status,
            ExternalMemoryProviderSyncStatus::NoRuntimeAdapter
        );
        assert!(!result.external_io_performed);
        assert_eq!(
            result.preflight.action,
            ExternalMemoryProviderPreflightAction::WouldSync
        );
        assert_eq!(
            result.error.as_deref(),
            Some("external memory provider runtime adapter is not wired")
        );
    }

    #[test]
    fn external_memory_provider_preflight_reports_stats_unavailable() {
        let stats = MemoryStats {
            total: 0,
            by_type: std::collections::HashMap::new(),
            by_source: std::collections::HashMap::new(),
            with_embedding: 0,
            oldest: None,
            newest: None,
        };
        let cfg = ExternalMemoryProvidersConfig {
            enabled: true,
            providers: vec![ExternalMemoryProviderConfig {
                id: "zep-main".to_string(),
                kind: ExternalMemoryProviderKind::Zep,
                display_name: "Zep".to_string(),
                enabled: true,
                sync_policy: ExternalMemorySyncPolicy::PushOnly,
                endpoint_configured: true,
                last_sync_at: None,
                last_error: None,
            }],
        };

        let report =
            cfg.sync_preflight_with_stats_status(&stats, Some("memory stats unavailable".into()));

        assert!(report.stats_unavailable);
        assert_eq!(
            report.stats_error.as_deref(),
            Some("memory stats unavailable")
        );
        assert_eq!(report.local_memory_total, 0);
        assert_eq!(report.local_memory_with_embedding, 0);
        assert_eq!(report.providers[0].local_memory_candidate_count, 0);
        assert_eq!(
            report.providers[0].action,
            ExternalMemoryProviderPreflightAction::WouldSync
        );
    }

    #[test]
    fn external_memory_provider_preflight_reports_would_sync_when_adapter_ready() {
        let stats = MemoryStats {
            total: 7,
            by_type: std::collections::HashMap::new(),
            by_source: std::collections::HashMap::new(),
            with_embedding: 0,
            oldest: None,
            newest: None,
        };
        let provider = ExternalMemoryProviderConfig {
            id: "custom-main".to_string(),
            kind: ExternalMemoryProviderKind::Custom,
            display_name: "Custom".to_string(),
            enabled: true,
            sync_policy: ExternalMemorySyncPolicy::PushOnly,
            endpoint_configured: true,
            last_sync_at: None,
            last_error: None,
        };
        let health = ExternalMemoryProviderHealth::from_config_with_capabilities(
            &provider,
            true,
            ExternalMemoryProviderCapabilities {
                adapter_available: true,
                requires_endpoint: true,
                supports_manual: true,
                supports_pull: true,
                supports_push: true,
                supports_bidirectional: false,
            },
        );

        let item = external_memory_provider_preflight(&provider, health, true, &stats);
        assert_eq!(
            item.action,
            ExternalMemoryProviderPreflightAction::WouldSync
        );
        assert_eq!(
            item.runtime_data_flow,
            ExternalMemoryProviderDataFlow::PushOnly
        );
        assert!(!item.runtime_sends_query_context);
        assert!(item.runtime_sends_local_memory);
        assert!(!item.runtime_imports_external_memory);
        assert_eq!(item.local_memory_candidate_count, 7);
    }

    #[test]
    fn external_memory_sync_policy_checks_provider_capabilities() {
        let capabilities = ExternalMemoryProviderCapabilities {
            adapter_available: true,
            requires_endpoint: true,
            supports_manual: true,
            supports_pull: true,
            supports_push: false,
            supports_bidirectional: false,
        };

        assert!(ExternalMemorySyncPolicy::Off.supported_by(&capabilities));
        assert!(ExternalMemorySyncPolicy::Manual.supported_by(&capabilities));
        assert!(ExternalMemorySyncPolicy::PullOnly.supported_by(&capabilities));
        assert!(!ExternalMemorySyncPolicy::PushOnly.supported_by(&capabilities));
        assert!(!ExternalMemorySyncPolicy::Bidirectional.supported_by(&capabilities));
    }

    #[test]
    fn external_memory_provider_health_reports_unsupported_policy_blocker() {
        let provider = ExternalMemoryProviderConfig {
            id: "mem0-main".to_string(),
            kind: ExternalMemoryProviderKind::Mem0,
            display_name: "Mem0".to_string(),
            enabled: true,
            sync_policy: ExternalMemorySyncPolicy::PushOnly,
            endpoint_configured: true,
            last_sync_at: None,
            last_error: None,
        };
        let health = ExternalMemoryProviderHealth::from_config_with_capabilities(
            &provider,
            true,
            ExternalMemoryProviderCapabilities {
                adapter_available: true,
                requires_endpoint: true,
                supports_manual: true,
                supports_pull: true,
                supports_push: false,
                supports_bidirectional: false,
            },
        );

        assert!(health.sync_blocked);
        assert_eq!(
            health.sync_block_reasons,
            vec![ExternalMemoryProviderSyncBlockReason::PolicyUnsupported]
        );
    }

    #[test]
    fn external_memory_provider_config_normalizes_bounded_non_secret_metadata() {
        let long_error = "x".repeat(MAX_EXTERNAL_PROVIDER_ERROR_CHARS + 10);
        let cfg = ExternalMemoryProvidersConfig {
            enabled: true,
            providers: vec![
                ExternalMemoryProviderConfig {
                    id: " Mem0/Main ".to_string(),
                    kind: ExternalMemoryProviderKind::Mem0,
                    display_name: "  ".to_string(),
                    enabled: true,
                    sync_policy: ExternalMemorySyncPolicy::Manual,
                    endpoint_configured: false,
                    last_sync_at: Some(" ".to_string()),
                    last_error: Some(long_error),
                },
                ExternalMemoryProviderConfig {
                    id: "mem0-main".to_string(),
                    kind: ExternalMemoryProviderKind::Mem0,
                    display_name: "Second".to_string(),
                    enabled: false,
                    sync_policy: ExternalMemorySyncPolicy::Off,
                    endpoint_configured: false,
                    last_sync_at: None,
                    last_error: None,
                },
            ],
        }
        .normalized();

        assert_eq!(cfg.providers[0].id, "mem0-main");
        assert_eq!(cfg.providers[0].display_name, "mem0");
        assert_eq!(
            cfg.providers[0]
                .last_error
                .as_ref()
                .unwrap()
                .chars()
                .count(),
            MAX_EXTERNAL_PROVIDER_ERROR_CHARS
        );
        assert!(cfg.providers[0].last_sync_at.is_none());
        assert_eq!(cfg.providers[1].id, "mem0-main-2");
    }
}
