//! Knowledge maintenance configuration — persisted under
//! `AppConfig.knowledge_maintenance`. Mirrors the dreaming config shape
//! (master switch + idle/cron triggers + manual), plus per-task toggles and an
//! auto-approve gate. **Disabled by default**: autonomous writes to a user's
//! notes are sensitive, so nothing runs until the user opts in.

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}
fn default_idle_minutes() -> u32 {
    45
}
fn default_cron_expr() -> String {
    // 6-field (sec min hour dom mon dow) for the `cron` crate — daily 03:30.
    "0 30 3 * * *".to_string()
}
fn default_max_proposals() -> usize {
    20
}
fn default_dedup_similarity() -> f32 {
    0.86
}
fn default_llm_timeout_secs() -> u64 {
    60
}
fn default_llm_max_tokens() -> u32 {
    2048
}

/// Idle trigger: generate proposals after the app has been idle this long.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceIdleTrigger {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_idle_minutes")]
    pub idle_minutes: u32,
}

impl Default for MaintenanceIdleTrigger {
    fn default() -> Self {
        Self {
            enabled: false,
            idle_minutes: default_idle_minutes(),
        }
    }
}

/// Cron trigger: generate proposals on a crontab schedule (6-field).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceCronTrigger {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_cron_expr")]
    pub cron_expr: String,
}

impl Default for MaintenanceCronTrigger {
    fn default() -> Self {
        Self {
            enabled: false,
            cron_expr: default_cron_expr(),
        }
    }
}

/// Per-task on/off switches. All on by default (gated by the master `enabled`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceTasks {
    #[serde(default = "default_true")]
    pub auto_link: bool,
    #[serde(default = "default_true")]
    pub orphan_rescue: bool,
    #[serde(default = "default_true")]
    pub frontmatter_fill: bool,
    #[serde(default = "default_true")]
    pub dedup_merge: bool,
    #[serde(default = "default_true")]
    pub knowledge_gap: bool,
    #[serde(default = "default_true")]
    pub auto_tag: bool,
    #[serde(default = "default_true")]
    pub moc_upkeep: bool,
    #[serde(default = "default_true")]
    pub memory_to_note: bool,
    #[serde(default = "default_true")]
    pub source_compile: bool,
    #[serde(default = "default_true")]
    pub source_conflict: bool,
    #[serde(default = "default_true")]
    pub open_questions_moc: bool,
    #[serde(default = "default_true")]
    pub for_agent_summary: bool,
}

impl Default for MaintenanceTasks {
    fn default() -> Self {
        Self {
            auto_link: true,
            orphan_rescue: true,
            frontmatter_fill: true,
            dedup_merge: true,
            knowledge_gap: true,
            auto_tag: true,
            moc_upkeep: true,
            memory_to_note: true,
            source_compile: true,
            source_conflict: true,
            open_questions_moc: true,
            for_agent_summary: true,
        }
    }
}

impl MaintenanceTasks {
    pub fn enabled_for(&self, kind: super::types::ProposalKind) -> bool {
        use super::types::ProposalKind as K;
        match kind {
            K::AutoLink => self.auto_link,
            K::OrphanRescue => self.orphan_rescue,
            K::FrontmatterFill => self.frontmatter_fill,
            K::DedupMerge => self.dedup_merge,
            K::KnowledgeGap => self.knowledge_gap,
            K::AutoTag => self.auto_tag,
            K::MocUpkeep => self.moc_upkeep,
            K::MemoryToNote => self.memory_to_note,
            K::SourceCompile => self.source_compile,
            K::SourceConflict => self.source_conflict,
            K::OpenQuestionsMoc => self.open_questions_moc,
            K::ForAgentSummary => self.for_agent_summary,
        }
    }
}

/// Top-level maintenance config. Persisted under `AppConfig.knowledge_maintenance`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceConfig {
    /// Master switch. When `false`, every trigger is a no-op. Default `false`.
    #[serde(default)]
    pub enabled: bool,

    #[serde(default)]
    pub idle_trigger: MaintenanceIdleTrigger,

    #[serde(default)]
    pub cron_trigger: MaintenanceCronTrigger,

    /// Whether the GUI "Run now" button works (independent of triggers).
    #[serde(default = "default_true")]
    pub manual_enabled: bool,

    #[serde(default)]
    pub tasks: MaintenanceTasks,

    /// When `true`, proposals are applied immediately instead of waiting in the
    /// review queue. Off by default — the whole point is human review.
    #[serde(default)]
    pub auto_approve: bool,

    /// Cap on proposals queued per cycle (across all tasks).
    #[serde(default = "default_max_proposals")]
    pub max_proposals_per_cycle: usize,

    /// Title-token Jaccard threshold (0–1) above which two notes are merge
    /// candidates (deterministic dedup; an exact content hash always qualifies).
    #[serde(default = "default_dedup_similarity")]
    pub dedup_similarity: f32,

    #[serde(default = "default_llm_timeout_secs")]
    pub llm_timeout_secs: u64,

    #[serde(default = "default_llm_max_tokens")]
    pub llm_max_tokens: u32,
}

impl Default for MaintenanceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            idle_trigger: MaintenanceIdleTrigger::default(),
            cron_trigger: MaintenanceCronTrigger::default(),
            manual_enabled: true,
            tasks: MaintenanceTasks::default(),
            auto_approve: false,
            max_proposals_per_cycle: default_max_proposals(),
            dedup_similarity: default_dedup_similarity(),
            llm_timeout_secs: default_llm_timeout_secs(),
            llm_max_tokens: default_llm_max_tokens(),
        }
    }
}

impl MaintenanceConfig {
    /// Clamp hand-edited values to safe ranges.
    pub fn clamped(&self) -> Self {
        let mut c = self.clone();
        c.max_proposals_per_cycle = c.max_proposals_per_cycle.clamp(1, 200);
        c.dedup_similarity = c.dedup_similarity.clamp(0.5, 0.99);
        c.llm_timeout_secs = c.llm_timeout_secs.clamp(5, 600);
        c.llm_max_tokens = c.llm_max_tokens.clamp(256, 8192);
        c.idle_trigger.idle_minutes = c.idle_trigger.idle_minutes.max(1);
        c
    }
}
