//! Knowledge Base types.
//!
//! Two storage classes, deliberately kept apart (design D9):
//!
//! - [`KnowledgeBase`] and the access bindings are the **truth source**, persisted
//!   in `sessions.db` next to `projects` (registry, cannot be rebuilt from disk).
//! - [`Note`] / [`NoteChunk`] / [`NoteLink`] / [`NoteTag`] are **rebuildable cache
//!   rows** living in `~/.hope-agent/knowledge/index.db` — every field, including
//!   `rel_path`, is reconstructable by re-scanning the `.md` files.
//!
//! Coordinate contract (design D14): persistent `*_offset` fields are **Unicode
//! code-point offsets** relative to the *original full file* (frontmatter +
//! original CRLF included). The cross-end UI positioning fields are `*_line`
//! (1-based) + `*_col` (0-based code-point column, tab counted as one). `body`
//! is normalized search text and is decoupled from the coordinates.

use serde::{Deserialize, Serialize};

pub const DEFAULT_SCHEMA_SECTIONS: [&str; 6] = [
    "For Agent",
    "Compiled Truth",
    "Timeline",
    "Evidence",
    "Open Questions",
    "Related",
];

// ── KnowledgeBase (truth source, sessions.db) ────────────────────

/// A knowledge base = a notes container with a single storage root.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeBase {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    /// Notes root absolute path. `None` = default internal
    /// `~/.hope-agent/knowledge/{id}/notes/` (lazy ensure). `Some(_)` = bound to
    /// an external directory (e.g. an Obsidian/Logseq vault) — **read-only by
    /// default** unless `allow_external_writes` is set (WS7, design D11).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_dir: Option<String>,
    /// Opt-in to writing an external (bound) root from the GUI / agent tools
    /// (WS7). Default `false` — external vaults are read-only unless the user
    /// explicitly unlocks editing. Ignored for internal KBs (always writable);
    /// background autonomous maintenance never writes external regardless.
    #[serde(default)]
    pub allow_external_writes: bool,
    #[serde(default)]
    pub archived: bool,
    /// Unix milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
}

impl KnowledgeBase {
    /// Whether this KB is bound to an external (out-of-app) directory.
    pub fn is_external(&self) -> bool {
        self.root_dir
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    }

    /// Whether the storage root rejects mutations: an external root that has not
    /// been opted into writes (WS7). Internal KBs are always writable.
    pub fn is_read_only_root(&self) -> bool {
        self.is_external() && !self.allow_external_writes
    }

    /// Emoji-prefixed label for pickers / IM bodies without a separate emoji slot.
    pub fn display_label(&self) -> String {
        match self.emoji.as_deref().filter(|e| !e.is_empty()) {
            Some(e) => format!("{} {}", e, self.name),
            None => self.name.clone(),
        }
    }
}

/// KnowledgeBase with aggregated counts for listing / UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeBaseMeta {
    #[serde(flatten)]
    pub kb: KnowledgeBase,
    /// Indexed note count (from index.db; the registry fills 0 and the command
    /// layer enriches it, mirroring `ProjectMeta::memory_count`).
    pub note_count: u32,
    /// Whether the KB root is external (browse-only in Phase 1).
    pub external: bool,
}

/// Access level granted to a session / project over a KB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KbAccess {
    /// `note_search` / `note_read` only.
    Read,
    /// Read + mutating tools (`note_create/update/patch/append/delete/link`).
    Write,
}

impl KbAccess {
    pub fn as_str(&self) -> &'static str {
        match self {
            KbAccess::Read => "read",
            KbAccess::Write => "write",
        }
    }

    pub fn from_str_lenient(s: &str) -> KbAccess {
        match s {
            "write" => KbAccess::Write,
            _ => KbAccess::Read,
        }
    }
}

// ── Input DTOs ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateKnowledgeBaseInput {
    pub name: String,
    #[serde(default)]
    pub emoji: Option<String>,
    /// External vault path. Empty string → `NULL` (internal default).
    #[serde(default)]
    pub root_dir: Option<String>,
}

/// Patch DTO. `None` = leave unchanged; empty/whitespace string clears to `NULL`
/// (same convention as `UpdateProjectInput`). `root_dir` is intentionally **not**
/// patchable after creation in Phase 1 — switching a KB's root would invalidate
/// the whole index; recreate the KB instead.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateKnowledgeBaseInput {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(default)]
    pub archived: Option<bool>,
    /// Unlock / re-lock writes to an external (bound) root (WS7). `None` = leave
    /// unchanged. Ignored for internal KBs.
    #[serde(default)]
    pub allow_external_writes: Option<bool>,
}

// ── Access bindings (truth source, sessions.db) ──────────────────

/// A KB visible to a session / project, with the granted access level. Returned
/// for the "currently effective knowledge bases" UI list (the last human-facing
/// line of defense against leakage, design D10).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbAttachment {
    #[serde(flatten)]
    pub kb: KnowledgeBase,
    pub access: KbAccess,
    /// `"session"` or `"project"` — where this attach was granted.
    pub via: String,
}

/// A composer-staged KB attach carried on the `chat` command and applied on the
/// auto-create branch (mirrors draft `working_dir`). `access` is lenient
/// (`"read"` / `"write"`). camelCase over the wire (`kbId`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbAttachInput {
    pub kb_id: String,
    pub access: String,
}

/// A note the chat composer can reference via `[[ ]]`, flattened across every KB
/// reachable from the current chat (session ∪ project attaches, or staged draft
/// attaches for a brand-new chat). Owner plane — the user picks their own notes;
/// `[[note]]` injection re-gates through `effective_kb_access` at send time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceableNote {
    pub kb_id: String,
    pub kb_name: String,
    pub kb_emoji: Option<String>,
    /// Path relative to the KB root, `/`-separated (token + secondary display).
    pub rel_path: String,
    /// frontmatter title > first H1 > file stem (primary display).
    pub title: String,
}

// ── Raw sources (Knowledge Compiler Phase 1, sessions.db truth source) ─────

/// A raw-source type in the Knowledge Compiler inbox. Raw sources are distinct
/// from compiled notes: sources are immutable-ish input snapshots, while notes
/// remain the editable `.md` wiki layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeSourceKind {
    Markdown,
    Text,
    Pdf,
    Docx,
    AudioTranscript,
    VideoTranscript,
    ImageOcr,
    BrowserSnapshot,
    UrlSnapshot,
}

impl KnowledgeSourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            KnowledgeSourceKind::Markdown => "markdown",
            KnowledgeSourceKind::Text => "text",
            KnowledgeSourceKind::Pdf => "pdf",
            KnowledgeSourceKind::Docx => "docx",
            KnowledgeSourceKind::AudioTranscript => "audio_transcript",
            KnowledgeSourceKind::VideoTranscript => "video_transcript",
            KnowledgeSourceKind::ImageOcr => "image_ocr",
            KnowledgeSourceKind::BrowserSnapshot => "browser_snapshot",
            KnowledgeSourceKind::UrlSnapshot => "url_snapshot",
        }
    }

    pub fn from_str_lenient(s: &str) -> KnowledgeSourceKind {
        match s {
            "markdown" => KnowledgeSourceKind::Markdown,
            "pdf" => KnowledgeSourceKind::Pdf,
            "docx" => KnowledgeSourceKind::Docx,
            "audio_transcript" | "audioTranscript" | "audio" => {
                KnowledgeSourceKind::AudioTranscript
            }
            "video_transcript" | "videoTranscript" | "video" => {
                KnowledgeSourceKind::VideoTranscript
            }
            "image_ocr" | "imageOcr" | "image" | "ocr" => KnowledgeSourceKind::ImageOcr,
            "browser_snapshot" | "browserSnapshot" | "browser" => {
                KnowledgeSourceKind::BrowserSnapshot
            }
            "url_snapshot" | "urlSnapshot" | "url" => KnowledgeSourceKind::UrlSnapshot,
            _ => KnowledgeSourceKind::Text,
        }
    }
}

/// Browser capture mode for Phase 9 source imports. `Auto` prefers the current
/// text selection when present and otherwise captures the page's readable body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeBrowserCaptureMode {
    #[default]
    Auto,
    Selection,
    Page,
}

/// Owner-plane import request for capturing the active controlled browser tab
/// into the raw-source inbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeBrowserSourceImportInput {
    #[serde(default)]
    pub mode: KnowledgeBrowserCaptureMode,
    #[serde(default)]
    pub title: Option<String>,
}

/// Lifecycle status for a raw source. Phase 1 creates only `Ready` rows on
/// successful import; the explicit enum keeps later extraction/retry states
/// forward-compatible without changing the wire shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeSourceStatus {
    Ready,
    Failed,
}

impl KnowledgeSourceStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            KnowledgeSourceStatus::Ready => "ready",
            KnowledgeSourceStatus::Failed => "failed",
        }
    }

    pub fn from_str_lenient(s: &str) -> KnowledgeSourceStatus {
        match s {
            "failed" => KnowledgeSourceStatus::Failed,
            _ => KnowledgeSourceStatus::Ready,
        }
    }
}

/// Import request for Phase 1 raw sources. Exactly one of `content` or `url`
/// must be supplied. File imports are intentionally text-over-JSON so desktop
/// and HTTP/server mode behave the same and no endpoint reads arbitrary host
/// paths.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSourceImportInput {
    #[serde(default)]
    pub kind: Option<KnowledgeSourceKind>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub data_base64: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSourceImportBatchItemInput {
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    pub input: KnowledgeSourceImportInput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSourceImportBatchInput {
    pub items: Vec<KnowledgeSourceImportBatchItemInput>,
}

/// Raw source metadata for inbox lists.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSource {
    pub id: String,
    pub kb_id: String,
    pub kind: KnowledgeSourceKind,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_uri: Option<String>,
    pub stored_path: String,
    pub content_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted_text_hash: Option<String>,
    pub status: KnowledgeSourceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compiled_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub chunk_count: u32,
}

/// Source read response: metadata + stored snapshot text.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSourceReadResult {
    #[serde(flatten)]
    pub source: KnowledgeSource,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeSourceImportRunStatus {
    Running,
    Completed,
    CompletedWithErrors,
    Failed,
}

impl KnowledgeSourceImportRunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            KnowledgeSourceImportRunStatus::Running => "running",
            KnowledgeSourceImportRunStatus::Completed => "completed",
            KnowledgeSourceImportRunStatus::CompletedWithErrors => "completed_with_errors",
            KnowledgeSourceImportRunStatus::Failed => "failed",
        }
    }

    pub fn from_str_lenient(s: &str) -> KnowledgeSourceImportRunStatus {
        match s {
            "completed" => KnowledgeSourceImportRunStatus::Completed,
            "completed_with_errors" | "completedWithErrors" => {
                KnowledgeSourceImportRunStatus::CompletedWithErrors
            }
            "failed" => KnowledgeSourceImportRunStatus::Failed,
            _ => KnowledgeSourceImportRunStatus::Running,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeSourceImportItemStatus {
    Pending,
    Running,
    Imported,
    Duplicate,
    Failed,
}

impl KnowledgeSourceImportItemStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            KnowledgeSourceImportItemStatus::Pending => "pending",
            KnowledgeSourceImportItemStatus::Running => "running",
            KnowledgeSourceImportItemStatus::Imported => "imported",
            KnowledgeSourceImportItemStatus::Duplicate => "duplicate",
            KnowledgeSourceImportItemStatus::Failed => "failed",
        }
    }

    pub fn from_str_lenient(s: &str) -> KnowledgeSourceImportItemStatus {
        match s {
            "running" => KnowledgeSourceImportItemStatus::Running,
            "imported" => KnowledgeSourceImportItemStatus::Imported,
            "duplicate" => KnowledgeSourceImportItemStatus::Duplicate,
            "failed" => KnowledgeSourceImportItemStatus::Failed,
            _ => KnowledgeSourceImportItemStatus::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSourceImportItem {
    pub id: i64,
    pub run_id: String,
    pub kb_id: String,
    pub position: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<KnowledgeSourceKind>,
    pub status: KnowledgeSourceImportItemStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicate_of_source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSourceImportRun {
    pub id: String,
    pub kb_id: String,
    pub status: KnowledgeSourceImportRunStatus,
    pub total_count: u32,
    pub imported_count: u32,
    pub duplicate_count: u32,
    pub failed_count: u32,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSourceImportRunDetail {
    #[serde(flatten)]
    pub run: KnowledgeSourceImportRun,
    pub items: Vec<KnowledgeSourceImportItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeSourceSimilarityGroupKind {
    ExactDuplicate,
    Similar,
}

impl KnowledgeSourceSimilarityGroupKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            KnowledgeSourceSimilarityGroupKind::ExactDuplicate => "exact_duplicate",
            KnowledgeSourceSimilarityGroupKind::Similar => "similar",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSourceSimilarityGroup {
    pub id: String,
    pub kind: KnowledgeSourceSimilarityGroupKind,
    pub similarity: f32,
    pub fingerprint: String,
    pub sources: Vec<KnowledgeSource>,
}

/// Separate chunk rows for raw sources. They never share `note_chunk`, keeping
/// source snapshots out of compiled-note ranking and prompt surfaces.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSourceChunk {
    pub id: i64,
    pub source_id: String,
    pub chunk_index: i64,
    pub body: String,
    pub start_offset: u32,
    pub end_offset: u32,
    pub content_hash: String,
}

// ── Schema Profile + Evidence (Knowledge Compiler Phase 3) ───────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaProfile {
    pub kb_id: String,
    pub page_types: Vec<SchemaPageTypeSpec>,
    pub default_page_type: String,
    pub required_sections: Vec<String>,
    pub updated_at: i64,
}

impl SchemaProfile {
    pub fn default_for(kb_id: &str, updated_at: i64) -> Self {
        Self {
            kb_id: kb_id.to_string(),
            page_types: vec![
                SchemaPageTypeSpec::new("source_summary", "Source Summary"),
                SchemaPageTypeSpec::new("conversation_note", "Conversation Note"),
                SchemaPageTypeSpec::new("concept", "Concept"),
                SchemaPageTypeSpec::new("person", "Person"),
                SchemaPageTypeSpec::new("project", "Project"),
                SchemaPageTypeSpec::new("decision", "Decision"),
                SchemaPageTypeSpec::new("timeline", "Timeline"),
                SchemaPageTypeSpec::new("moc", "Map of Content"),
            ],
            default_page_type: "source_summary".to_string(),
            required_sections: DEFAULT_SCHEMA_SECTIONS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaPageTypeSpec {
    pub key: String,
    pub label: String,
    pub required_sections: Vec<String>,
    pub required_frontmatter: Vec<String>,
}

impl SchemaPageTypeSpec {
    pub fn new(key: &str, label: &str) -> Self {
        Self {
            key: key.to_string(),
            label: label.to_string(),
            required_sections: DEFAULT_SCHEMA_SECTIONS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            required_frontmatter: vec![
                "type".to_string(),
                "sources".to_string(),
                "last_compiled".to_string(),
                "confidence".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaIssueKind {
    MissingEvidence,
    StaleSource,
    SchemaViolation,
    ConflictingClaim,
    UnfiledOpenQuestion,
}

impl SchemaIssueKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SchemaIssueKind::MissingEvidence => "missing_evidence",
            SchemaIssueKind::StaleSource => "stale_source",
            SchemaIssueKind::SchemaViolation => "schema_violation",
            SchemaIssueKind::ConflictingClaim => "conflicting_claim",
            SchemaIssueKind::UnfiledOpenQuestion => "unfiled_open_question",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaIssue {
    pub kb_id: String,
    pub rel_path: String,
    pub title: String,
    pub kind: SchemaIssueKind,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteSourceRef {
    pub source_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_uri: Option<String>,
    pub missing: bool,
    pub stale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_updated_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_last_compiled_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cited_in: Vec<String>,
}

// ── Phase 6 external-agent API ──────────────────────────────────

/// Stable item discriminator for external agents. `compiled_note` means the
/// result is a normal wiki note with source/evidence markers; raw sources never
/// masquerade as notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeAgentItemKind {
    Note,
    CompiledNote,
    Source,
}

/// `knowledge.search` input. Notes are always searched first; raw sources are
/// included only when explicitly requested.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeAgentSearchInput {
    pub query: String,
    #[serde(default)]
    pub kb_id: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub include_sources: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeAgentNoteHit {
    pub kind: KnowledgeAgentItemKind,
    pub kb_id: String,
    #[serde(default)]
    pub kb_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kb_emoji: Option<String>,
    pub note_id: i64,
    pub rel_path: String,
    pub title: String,
    pub score: f32,
    pub snippet: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_path: Option<String>,
    pub start_line: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeAgentSourceItem {
    pub kind: KnowledgeAgentItemKind,
    pub kb_id: String,
    pub source_id: String,
    pub source_kind: KnowledgeSourceKind,
    pub status: KnowledgeSourceStatus,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_uri: Option<String>,
    pub content_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compiled_at: Option<i64>,
    pub stale: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub size: i64,
    pub chunk_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeAgentSearchResult {
    pub notes: Vec<KnowledgeAgentNoteHit>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<KnowledgeAgentSourceItem>,
    #[serde(default)]
    pub truncated: bool,
}

/// `knowledge.read` input. Exactly one of `path` or `reference` should be
/// supplied.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeAgentReadInput {
    pub kb_id: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub include_source_refs: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeAgentReadResult {
    pub kind: KnowledgeAgentItemKind,
    pub kb_id: String,
    pub note_id: i64,
    pub rel_path: String,
    pub title: String,
    pub content: String,
    pub content_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontmatter_json: Option<String>,
    pub outgoing_links: Vec<NoteLink>,
    pub backlinks: Vec<Backlink>,
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<NoteSourceRef>,
}

/// `knowledge.expand` input: read one note plus adjacent context that external
/// agents can choose to follow.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeAgentExpandInput {
    pub kb_id: String,
    pub path: String,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeAgentExpandResult {
    pub note: KnowledgeAgentReadResult,
    pub related_notes: Vec<KnowledgeAgentNoteHit>,
}

/// `knowledge.sources` input. Listing returns metadata/snippets; source content
/// is returned only for an explicit `source_id` plus `include_content=true`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeAgentSourcesInput {
    pub kb_id: String,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub include_content: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeAgentSourcesResult {
    pub sources: Vec<KnowledgeAgentSourceItem>,
    #[serde(default)]
    pub truncated: bool,
}

/// `knowledge.compile.propose` input. This starts the normal compile run and
/// creates review proposals; it never applies note writes by itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeAgentCompileProposeInput {
    pub kb_id: String,
    pub source_ids: Vec<String>,
    #[serde(default)]
    pub strategy: Option<String>,
}

// ── Knowledge Compiler Phase 2 ──────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompileRunStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl CompileRunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CompileRunStatus::Running => "running",
            CompileRunStatus::Completed => "completed",
            CompileRunStatus::Failed => "failed",
            CompileRunStatus::Cancelled => "cancelled",
        }
    }

    pub fn from_str_lenient(s: &str) -> CompileRunStatus {
        match s {
            "completed" => CompileRunStatus::Completed,
            "failed" => CompileRunStatus::Failed,
            "cancelled" => CompileRunStatus::Cancelled,
            _ => CompileRunStatus::Running,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompileProposalStatus {
    Draft,
    Applied,
    Rejected,
    Failed,
}

impl CompileProposalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CompileProposalStatus::Draft => "draft",
            CompileProposalStatus::Applied => "applied",
            CompileProposalStatus::Rejected => "rejected",
            CompileProposalStatus::Failed => "failed",
        }
    }

    pub fn from_str_lenient(s: &str) -> CompileProposalStatus {
        match s {
            "applied" => CompileProposalStatus::Applied,
            "rejected" => CompileProposalStatus::Rejected,
            "failed" => CompileProposalStatus::Failed,
            _ => CompileProposalStatus::Draft,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompileProposalKind {
    CreateNote,
    PatchNote,
    SetFrontmatter,
    AppendLink,
    CreateMoc,
}

impl CompileProposalKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CompileProposalKind::CreateNote => "create_note",
            CompileProposalKind::PatchNote => "patch_note",
            CompileProposalKind::SetFrontmatter => "set_frontmatter",
            CompileProposalKind::AppendLink => "append_link",
            CompileProposalKind::CreateMoc => "create_moc",
        }
    }

    pub fn from_str_lenient(s: &str) -> CompileProposalKind {
        match s {
            "patch_note" => CompileProposalKind::PatchNote,
            "set_frontmatter" => CompileProposalKind::SetFrontmatter,
            "append_link" => CompileProposalKind::AppendLink,
            "create_moc" => CompileProposalKind::CreateMoc,
            _ => CompileProposalKind::CreateNote,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum CompileProposalAction {
    CreateNote {
        path: String,
        content: String,
        #[serde(default)]
        overwrite: bool,
    },
    PatchNote {
        path: String,
        old: String,
        new: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_file_hash: Option<String>,
    },
    SetFrontmatter {
        path: String,
        props: serde_json::Map<String, serde_json::Value>,
    },
    AppendLink {
        from_path: String,
        to_ref: String,
    },
    CreateMoc {
        path: String,
        content: String,
        #[serde(default)]
        overwrite: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileStartInput {
    pub source_ids: Vec<String>,
    #[serde(default)]
    pub strategy: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryFileMode {
    CreateNote,
    UpdateCurrentNote,
    AppendToMoc,
    AppendOpenQuestions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryFileInput {
    pub session_id: String,
    pub message_id: i64,
    #[serde(default)]
    pub mode: Option<QueryFileMode>,
    #[serde(default)]
    pub current_note_path: Option<String>,
    #[serde(default)]
    pub target_path: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    /// Required when filing from a non-knowledge chat surface so the caller must
    /// explicitly acknowledge that chat content will be written into a KB.
    #[serde(default)]
    pub confirm_conversation_source: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileRun {
    pub id: String,
    pub kb_id: String,
    pub status: CompileRunStatus,
    pub source_ids: Vec<String>,
    pub strategy: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_label: Option<String>,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub proposal_count: u32,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewCompileProposal {
    pub kind: CompileProposalKind,
    pub title: String,
    pub detail: String,
    pub action: CompileProposalAction,
    pub fingerprint: String,
    pub source_ids: Vec<String>,
    pub before_text: Option<String>,
    pub after_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileProposal {
    pub id: i64,
    pub run_id: String,
    pub kb_id: String,
    pub kind: CompileProposalKind,
    pub status: CompileProposalStatus,
    pub title: String,
    pub detail: String,
    pub action: CompileProposalAction,
    pub fingerprint: String,
    pub source_ids: Vec<String>,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_text: Option<String>,
}

// ── Note (index cache rows, index.db) ────────────────────────────

/// File-level note metadata. The real content lives in the `.md` file; this row
/// is rebuildable. Body search/embedding is delegated to [`NoteChunk`] (D12).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: i64,
    pub kb_id: String,
    /// Path relative to the KB root, `/`-separated.
    pub rel_path: String,
    /// frontmatter `title` > first H1 > file stem.
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontmatter_json: Option<String>,
    pub mtime: i64,
    pub size: i64,
    /// Whole-file BLAKE3 over raw bytes (no newline normalization, CRLF kept).
    /// A "most-recent-index token" for optimistic concurrency — **never** the
    /// write-time staleness source (that re-hashes the disk file). See the
    /// stale-write guard contract in [`crate::knowledge`].
    pub content_hash: String,
}

/// chunk-level retrieval unit. FTS5 + vec0 both index this table (D12).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteChunk {
    pub id: i64,
    pub note_id: i64,
    pub chunk_index: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_path: Option<String>,
    /// Normalized search text (frontmatter stripped). Not used for coordinates.
    pub body: String,
    /// Unicode code-point offsets relative to the original full file (D14).
    pub start_offset: u32,
    pub end_offset: u32,
    /// Cross-end UI positioning (D14): 1-based line, 0-based code-point column.
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    /// chunk content hash, drives per-chunk incremental re-embedding.
    pub content_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_signature: Option<String>,
}

/// Link type for a [`NoteLink`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkType {
    /// `[[ ]]`
    Wiki,
    /// `![[ ]]` (Phase 2)
    Embed,
    /// standard `[]()`
    Md,
}

impl LinkType {
    pub fn as_str(&self) -> &'static str {
        match self {
            LinkType::Wiki => "wiki",
            LinkType::Embed => "embed",
            LinkType::Md => "md",
        }
    }

    pub fn from_str_lenient(s: &str) -> LinkType {
        match s {
            "embed" => LinkType::Embed,
            "md" => LinkType::Md,
            _ => LinkType::Wiki,
        }
    }
}

/// A directed wikilink edge. Backlinks = rows where `target_note_id = ?`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteLink {
    pub src_note_id: i64,
    /// Raw target inside `[[ ]]` (title or `folder/note` path form).
    pub target_ref: String,
    /// Resolved target; `None` = dangling/broken link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_note_id: Option<i64>,
    pub link_type: LinkType,
    /// `[[Note#Heading]]` slug, or `^block-id` (Phase 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    /// `[[note|alias]]` display alias.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// Link source text (e.g. `[[folder/note#H|alias]]`), for UI / backlink context.
    pub raw_text: String,
    /// Link position inside the source file (D14 coords) — backlink jump target.
    pub src_start_line: u32,
    pub src_start_col: u32,
    pub src_end_line: u32,
    pub src_end_col: u32,
    /// Heading section the link sits under, for backlink context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src_heading_path: Option<String>,
}

// ── Search result types ──────────────────────────────────────────

/// A note returned by `note_search`, with the best-matching chunk snippet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteSearchHit {
    pub kb_id: String,
    /// Human-readable source KB name (resolved from the registry — index.db only
    /// stores `kb_id`, D9). Empty when unresolved; callers fall back to `kb_id`.
    /// Lets every recall path attribute a hit to its knowledge space (the point in
    /// multi-KB sessions). Filled by [`search::enrich_kb_names`].
    #[serde(default)]
    pub kb_name: String,
    /// Source KB emoji, if any (for compact source badges in the UI).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kb_emoji: Option<String>,
    pub note_id: i64,
    pub rel_path: String,
    pub title: String,
    /// Fused relevance score (RRF, post-decay) of the best chunk.
    pub score: f32,
    /// Best-chunk snippet (already truncated).
    pub snippet: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_path: Option<String>,
    /// Best-chunk start line (1-based) for jump-to.
    pub start_line: u32,
}

/// A broken (dangling) link: a `[[ ]]` whose target resolves to nothing. Carries
/// the source note + exact occurrence for jump-to + the unresolved `target_ref`
/// so the UI can offer "create this note" (design: `note_broken_links`, Phase 2).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrokenLink {
    pub src_note_id: i64,
    pub src_rel_path: String,
    pub src_title: String,
    /// The unresolved target inside `[[ ]]` (title or `folder/note`).
    pub target_ref: String,
    pub raw_text: String,
    pub src_start_line: u32,
    pub src_start_col: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src_heading_path: Option<String>,
}

/// Outcome of a note/folder rename or move that also rewrites inbound `[[ ]]`
/// links in other notes (design `note_rename`/`note_move`, #9). `filesChanged`
/// counts distinct source notes whose link text was updated; `linksRewritten`
/// counts individual link occurrences.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameOutcome {
    /// The normalized new relative path of the renamed note/folder.
    pub new_rel: String,
    pub files_changed: usize,
    pub links_rewritten: usize,
}

// ── Graph types (WS1, Phase 2) ───────────────────────────────────

/// One node in a knowledge-base link graph = a note. `inDegree` / `outDegree`
/// are computed over **resolved** edges (broken links contribute nothing), so a
/// node with both at 0 is an orphan (an island the UI colours distinctly).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub id: i64,
    pub rel_path: String,
    pub title: String,
    pub in_degree: u32,
    pub out_degree: u32,
}

/// One directed edge `source → target` (note ids), from a resolved `[[ ]]` /
/// `![[ ]]` link. Parallel links between the same pair are collapsed to one edge.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub source: i64,
    pub target: i64,
}

/// A user-pinned graph node position (Batch J). Keyed by **`rel_path`** (stable
/// across index rebuilds — `index.db` ids churn, so they must not be the key),
/// persisted in `sessions.db` (truth source, D9), not the rebuildable index.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNodePosition {
    pub rel_path: String,
    pub x: f64,
    pub y: f64,
}

/// A knowledge-space sidebar conversation thread. One row per
/// `kind='knowledge'` session, joined with session metadata for the history
/// picker (title / recency / size). `anchorNotePath` is the note that was open
/// when the conversation was created — used to default-load "the latest
/// conversation about this note".
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbChatThread {
    pub session_id: String,
    pub kb_id: String,
    pub anchor_note_path: Option<String>,
    /// Agent baked into this thread's session — restored when the history picker
    /// switches to it so follow-ups run with the thread's own agent + model.
    pub agent_id: String,
    /// Session title (LLM- or user-set), `None` until named.
    pub title: Option<String>,
    /// Thread creation time (epoch ms).
    pub created_at: i64,
    /// Session `updated_at` (rfc3339) — recency sort key for the picker.
    pub updated_at: String,
    /// Count of persisted messages (user + assistant + tool rows).
    pub message_count: i64,
    /// Last user/assistant message preview for the picker (trimmed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_snippet: Option<String>,
}

/// A note link graph (whole KB or an ego neighbourhood). `truncated` is set when
/// a node cap dropped part of the graph (agent tool guard against huge output).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    #[serde(default)]
    pub truncated: bool,
}

/// A backlink with enough context to jump to the exact link occurrence.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backlink {
    pub src_note_id: i64,
    pub src_rel_path: String,
    pub src_title: String,
    pub raw_text: String,
    pub src_start_line: u32,
    pub src_start_col: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src_heading_path: Option<String>,
}

/// Full note read result: raw content + outgoing links + backlinks + tags.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteReadResult {
    pub kb_id: String,
    pub note_id: i64,
    pub rel_path: String,
    pub title: String,
    /// Raw file content (markdown including frontmatter).
    pub content: String,
    pub content_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontmatter_json: Option<String>,
    pub outgoing_links: Vec<NoteLink>,
    pub backlinks: Vec<Backlink>,
    pub tags: Vec<String>,
}

/// Read bridge ③ — passive related-notes prompt (Phase 3, D7). Persisted in
/// `AppConfig.knowledge_passive_recall`. When enabled, each user turn searches
/// the accessible KBs by the user's message and injects the top note **titles**
/// as an independent, untrusted cache block (mirrors Active Memory's slot but
/// retrieval-only — no LLM call). Opt-in; disabled by default.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PassiveRecallConfig {
    /// Master switch. Off by default — turns surface related notes only after the
    /// user opts in (Settings → Knowledge).
    #[serde(default)]
    pub enabled: bool,
    /// Max related notes to list per turn.
    #[serde(default = "default_passive_top_n")]
    pub top_n: usize,
    /// Hard cap on the rendered block size (code points), defensively truncated.
    #[serde(default = "default_passive_max_chars")]
    pub max_chars: usize,
    /// Reuse the same retrieval for repeated identical messages within this window.
    #[serde(default = "default_passive_cache_ttl_secs")]
    pub cache_ttl_secs: u64,
    /// Include a one-line snippet under each title (more tokens; default titles-only).
    #[serde(default)]
    pub show_snippet: bool,
}

fn default_passive_top_n() -> usize {
    5
}
fn default_passive_max_chars() -> usize {
    800
}
fn default_passive_cache_ttl_secs() -> u64 {
    120
}

impl Default for PassiveRecallConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            top_n: default_passive_top_n(),
            max_chars: default_passive_max_chars(),
            cache_ttl_secs: default_passive_cache_ttl_secs(),
            show_snippet: false,
        }
    }
}

impl PassiveRecallConfig {
    /// Clamp to sane bounds so a hand-edited config can't blow up the prompt:
    /// `top_n` in `[1, 20]`, `max_chars` in `[100, 4000]`, `cache_ttl_secs` ≥ 1.
    pub fn clamped(&self) -> PassiveRecallConfig {
        PassiveRecallConfig {
            enabled: self.enabled,
            top_n: self.top_n.clamp(1, 20),
            max_chars: self.max_chars.clamp(100, 4000),
            cache_ttl_secs: self.cache_ttl_secs.max(1),
            show_snippet: self.show_snippet,
        }
    }
}
