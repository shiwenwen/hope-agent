//! Knowledge Base 配置类型（自 ha-core `knowledge::types` 下沉的
//! `AppConfig` 闭包子集）：媒体保留 / 源导入限额 / compile / vision / note
//! 工具 / 被动召回。非配置的领域类型（Note / KnowledgeSource / 提案 …）仍在
//! ha-core。

use serde::{Deserialize, Serialize};

const MEDIA_RETENTION_DEFAULT_MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MEDIA_RETENTION_DEFAULT_MAX_SOURCE_BYTES: u64 = 100 * 1024 * 1024;
const MEDIA_RETENTION_DEFAULT_THUMBNAIL_MAX_EDGE_PX: u32 = 512;
const MEDIA_RETENTION_MIN_TOTAL_BYTES: u64 = 10 * 1024 * 1024;
const MEDIA_RETENTION_MAX_TOTAL_BYTES: u64 = 100 * 1024 * 1024 * 1024;
const MEDIA_RETENTION_MIN_SOURCE_BYTES: u64 = 1024 * 1024;
const MEDIA_RETENTION_MAX_SOURCE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MEDIA_RETENTION_MIN_THUMBNAIL_EDGE_PX: u32 = 128;
const MEDIA_RETENTION_MAX_THUMBNAIL_EDGE_PX: u32 = 2048;

fn default_media_retention_max_total_bytes() -> u64 {
    MEDIA_RETENTION_DEFAULT_MAX_TOTAL_BYTES
}

fn default_media_retention_max_source_bytes() -> u64 {
    MEDIA_RETENTION_DEFAULT_MAX_SOURCE_BYTES
}

fn default_media_retention_thumbnail_max_edge_px() -> u32 {
    MEDIA_RETENTION_DEFAULT_THUMBNAIL_MAX_EDGE_PX
}

fn default_media_retention_prune_when_over_quota() -> bool {
    true
}

/// Privacy-gated optional retention for original media imported into raw
/// sources. Disabled by default; text snapshots remain the durable source of
/// truth even when this is off.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeMediaRetentionConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_media_retention_max_total_bytes")]
    pub max_total_bytes: u64,
    #[serde(default = "default_media_retention_max_source_bytes")]
    pub max_source_bytes: u64,
    #[serde(default = "default_media_retention_thumbnail_max_edge_px")]
    pub thumbnail_max_edge_px: u32,
    #[serde(default = "default_media_retention_prune_when_over_quota")]
    pub prune_when_over_quota: bool,
}

impl Default for KnowledgeMediaRetentionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_total_bytes: MEDIA_RETENTION_DEFAULT_MAX_TOTAL_BYTES,
            max_source_bytes: MEDIA_RETENTION_DEFAULT_MAX_SOURCE_BYTES,
            thumbnail_max_edge_px: MEDIA_RETENTION_DEFAULT_THUMBNAIL_MAX_EDGE_PX,
            prune_when_over_quota: true,
        }
    }
}

impl KnowledgeMediaRetentionConfig {
    pub fn clamped(mut self) -> Self {
        self.max_total_bytes = self.max_total_bytes.clamp(
            MEDIA_RETENTION_MIN_TOTAL_BYTES,
            MEDIA_RETENTION_MAX_TOTAL_BYTES,
        );
        self.max_source_bytes = self.max_source_bytes.clamp(
            MEDIA_RETENTION_MIN_SOURCE_BYTES,
            MEDIA_RETENTION_MAX_SOURCE_BYTES,
        );
        if self.max_source_bytes > self.max_total_bytes {
            self.max_source_bytes = self.max_total_bytes;
        }
        self.thumbnail_max_edge_px = self.thumbnail_max_edge_px.clamp(
            MEDIA_RETENTION_MIN_THUMBNAIL_EDGE_PX,
            MEDIA_RETENTION_MAX_THUMBNAIL_EDGE_PX,
        );
        self
    }
}

pub const DEFAULT_MAX_TEXT_SOURCE_MB: u32 = 5;
pub const MIN_MAX_TEXT_SOURCE_MB: u32 = 1;
pub const MAX_MAX_TEXT_SOURCE_MB: u32 = 20;
pub const DEFAULT_MAX_BINARY_SOURCE_MB: u32 = 24;
pub const MIN_MAX_BINARY_SOURCE_MB: u32 = 1;
pub const MAX_MAX_BINARY_SOURCE_MB: u32 = 100;
pub const DEFAULT_MAX_URL_RESPONSE_MB: u32 = 2;
pub const MIN_MAX_URL_RESPONSE_MB: u32 = 1;
pub const MAX_MAX_URL_RESPONSE_MB: u32 = 20;

fn default_max_text_source_mb() -> u32 {
    DEFAULT_MAX_TEXT_SOURCE_MB
}

fn default_max_binary_source_mb() -> u32 {
    DEFAULT_MAX_BINARY_SOURCE_MB
}

fn default_max_url_response_mb() -> u32 {
    DEFAULT_MAX_URL_RESPONSE_MB
}

/// User-configurable source-ingestion limits. Values are stored as MiB.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSourceLimitsConfig {
    #[serde(default = "default_max_text_source_mb")]
    pub max_text_source_mb: u32,
    #[serde(default = "default_max_binary_source_mb")]
    pub max_binary_source_mb: u32,
    #[serde(default = "default_max_url_response_mb")]
    pub max_url_response_mb: u32,
}

impl Default for KnowledgeSourceLimitsConfig {
    fn default() -> Self {
        Self {
            max_text_source_mb: DEFAULT_MAX_TEXT_SOURCE_MB,
            max_binary_source_mb: DEFAULT_MAX_BINARY_SOURCE_MB,
            max_url_response_mb: DEFAULT_MAX_URL_RESPONSE_MB,
        }
    }
}

impl KnowledgeSourceLimitsConfig {
    pub fn clamped(mut self) -> Self {
        self.max_text_source_mb = self
            .max_text_source_mb
            .clamp(MIN_MAX_TEXT_SOURCE_MB, MAX_MAX_TEXT_SOURCE_MB);
        self.max_binary_source_mb = self
            .max_binary_source_mb
            .clamp(MIN_MAX_BINARY_SOURCE_MB, MAX_MAX_BINARY_SOURCE_MB);
        self.max_url_response_mb = self
            .max_url_response_mb
            .clamp(MIN_MAX_URL_RESPONSE_MB, MAX_MAX_URL_RESPONSE_MB);
        self
    }

    pub fn max_text_source_bytes(&self) -> u64 {
        self.max_text_source_mb
            .clamp(MIN_MAX_TEXT_SOURCE_MB, MAX_MAX_TEXT_SOURCE_MB) as u64
            * 1024
            * 1024
    }

    pub fn max_binary_source_bytes(&self) -> u64 {
        self.max_binary_source_mb
            .clamp(MIN_MAX_BINARY_SOURCE_MB, MAX_MAX_BINARY_SOURCE_MB) as u64
            * 1024
            * 1024
    }

    pub fn max_url_response_bytes(&self) -> u64 {
        self.max_url_response_mb
            .clamp(MIN_MAX_URL_RESPONSE_MB, MAX_MAX_URL_RESPONSE_MB) as u64
            * 1024
            * 1024
    }
}

/// Model selection for source-to-note compile summaries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeCompileConfig {
    /// Deprecated — superseded by `model_override`. Agent id whose model
    /// config was borrowed for compile summaries. Kept for backward
    /// compatibility: still resolved to an equivalent `ModelChain` when
    /// `model_override` is unset, but the GUI no longer writes this field.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Model chain override for compile summaries. `None` = fall through to
    /// `function_models.automation` (or the deprecated `agent_id`, if still
    /// set) → chat default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<crate::provider::ModelChain>,
}

impl KnowledgeCompileConfig {
    pub fn normalized(mut self) -> Self {
        self.agent_id = self.agent_id.and_then(|id| {
            let trimmed = id.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
        self
    }
}

fn default_knowledge_vision_timeout_secs() -> u64 {
    90
}
fn default_knowledge_vision_max_tokens() -> u32 {
    4096
}
fn default_knowledge_ocr_concurrency() -> u8 {
    3
}
fn default_knowledge_ocr_max_pages() -> usize {
    40
}

/// Model selection for Knowledge's vision-capable ingestion paths — image OCR
/// import today. Named `Vision`, not `Ocr`: a future scanned-PDF-page OCR
/// fallback would share this same config rather than growing its own, since
/// both are "vision-transcribe an image for KB ingestion," just with a
/// different image source. No deprecated legacy field — OCR never had
/// dedicated config before this; it silently inherited whatever
/// `recap.analysis_agent`/the default agent resolved to, an orphaned-config
/// bug this field fixes rather than perpetuates.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeVisionConfig {
    /// `None` = fall through to `function_models.automation` → chat default,
    /// filtered to vision-capable candidates only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<crate::provider::ModelChain>,
    /// Total budget across every candidate `automation::run_vision` tries.
    /// OCR had no timeout at all before this — a hung first candidate would
    /// block trying the rest.
    #[serde(default = "default_knowledge_vision_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_knowledge_vision_max_tokens")]
    pub max_tokens: u32,
    /// Bounded concurrency for the scanned-PDF OCR fallback's per-page
    /// `run_vision` calls (`futures::stream::buffer_unordered`, mirrors the
    /// pattern already used by `recap/facets.rs`). Not used by the
    /// single-image OCR path (`ocr_image_bytes`), which is one call.
    #[serde(default = "default_knowledge_ocr_concurrency")]
    pub ocr_concurrency: u8,
    /// Page cap for the scanned-PDF OCR fallback. Higher than the chat
    /// attachment/vision-bridge defaults (8-10) on purpose: a knowledge-base
    /// import is a deliberate one-time archiving action, not inline
    /// conversation context, so it can afford a materially larger budget.
    #[serde(default = "default_knowledge_ocr_max_pages")]
    pub max_ocr_pages: usize,
}

impl Default for KnowledgeVisionConfig {
    fn default() -> Self {
        Self {
            model_override: None,
            timeout_secs: default_knowledge_vision_timeout_secs(),
            max_tokens: default_knowledge_vision_max_tokens(),
            ocr_concurrency: default_knowledge_ocr_concurrency(),
            max_ocr_pages: default_knowledge_ocr_max_pages(),
        }
    }
}

impl KnowledgeVisionConfig {
    /// Clamp hand-edited values to safe ranges (mirrors `MaintenanceConfig`/
    /// `SpriteConfig`'s `clamped()` — a skill/HTTP write shouldn't be able to
    /// persist a zero/absurd timeout or an unbounded token budget).
    pub fn clamped(&self) -> Self {
        let mut c = self.clone();
        c.timeout_secs = c.timeout_secs.clamp(10, 600);
        c.max_tokens = c.max_tokens.clamp(256, 8192);
        c.ocr_concurrency = c.ocr_concurrency.clamp(1, 8);
        c.max_ocr_pages = c.max_ocr_pages.clamp(1, 120);
        c
    }
}

/// Model selection for the standalone note-authoring tools (`note_distill`,
/// `note_moc`, `session_to_note`) — one shared field, not three independent
/// ones: all three already funnel through the same code chokepoint
/// (`tools::note::run_kb_side_query`) and had zero dedicated config before
/// this (implicitly rode on `recap.analysis_agent`), so there's no existing
/// per-tool precedent to preserve or diverge from. Homed in `knowledge::types`
/// (not `tools::note`, which is `pub(crate)` and unreachable from
/// `src-tauri`/`ha-server`'s Tauri/HTTP command signatures) alongside its
/// domain siblings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteToolsConfig {
    /// `None` = fall through to `function_models.automation` → chat default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<crate::provider::ModelChain>,
}

/// Read bridge ③ — passive related-notes prompt (Phase 3, D7). Persisted in
/// `AppConfig.knowledge_passive_recall`. When enabled, each user turn searches
/// the accessible KBs by the user's message and injects the top note **titles**
/// as an independent, untrusted cache block (mirrors Active Memory's slot but
/// retrieval-only — no LLM call). Enabled by default after KB access is granted;
/// access gates still enforce incognito / IM opt-in / attached-KB limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PassiveRecallConfig {
    /// Master switch. On by default because it is retrieval-only and still bounded
    /// by KB access; users can disable it in Settings → Knowledge.
    #[serde(default = "default_passive_recall_enabled")]
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

// `default_passive_top_n` / `default_passive_max_chars` /
// `default_passive_cache_ttl_secs` 为 pub：ha-core 侧测试引用（可见性升级，
// 原为模块私有）。
pub fn default_passive_top_n() -> usize {
    5
}
fn default_passive_recall_enabled() -> bool {
    true
}
pub fn default_passive_max_chars() -> usize {
    800
}
pub fn default_passive_cache_ttl_secs() -> u64 {
    120
}

impl Default for PassiveRecallConfig {
    fn default() -> Self {
        Self {
            enabled: default_passive_recall_enabled(),
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
