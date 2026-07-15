//! Application configuration — the root structure persisted to `~/.hope-agent/config.json`.
//!
//! Historically named `ProviderStore`, this type actually owns the entire
//! user-facing config (providers, channels, memory, skills, tools, UI, server…).
//! It was renamed to `AppConfig` to match its real scope.
//!
//! The on-disk JSON shape is unchanged — all fields use `#[serde(rename_all = "camelCase")]`
//! and no wrapper struct is involved, so the Rust type name has zero impact on serialization.

mod persistence;

#[cfg(test)]
pub use persistence::replace_cache_for_test;
pub use persistence::{
    cached_config, config_health, load_config, mutate_config, mutate_config_async,
    reload_cache_from_disk, save_config, ConfigHealth,
};

use serde::{Deserialize, Serialize};

use crate::provider::{ActiveModel, ModelChain, ProviderConfig, ProxyConfig};

// ── Shortcut Config ─────────────────────────────────────────────

/// A single keyboard shortcut binding
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutBinding {
    /// Unique identifier for this shortcut action
    pub id: String,
    /// The shortcut key combination (e.g. "Alt+Space", "CommandOrControl+Shift+K")
    /// Empty string means disabled.
    pub keys: String,
    /// Whether this shortcut is enabled
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
}

/// Global shortcut configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutConfig {
    /// All shortcut bindings
    #[serde(default = "default_shortcut_bindings")]
    pub bindings: Vec<ShortcutBinding>,
}

fn default_shortcut_bindings() -> Vec<ShortcutBinding> {
    vec![ShortcutBinding {
        id: "quickChat".to_string(),
        keys: "Alt+Space".to_string(),
        enabled: true,
    }]
}

impl ShortcutBinding {
    /// Whether this binding is a chord (two sequential key combos separated by space).
    /// e.g. "CommandOrControl+K CommandOrControl+C"
    pub fn is_chord(&self) -> bool {
        self.chord_parts().len() > 1
    }

    /// Split keys into chord parts. Single combo returns vec of 1.
    pub fn chord_parts(&self) -> Vec<&str> {
        self.keys.split_whitespace().collect()
    }
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        Self {
            bindings: default_shortcut_bindings(),
        }
    }
}

// ── Timeout Policy Config ────────────────────────────────────────

/// How model-supplied runtime timeout overrides are handled.
///
/// These are the timeout arguments that can shorten or kill a long-running unit
/// of work (`exec.timeout`, async `job_timeout_secs`, sub-agent timeouts, ACP
/// run timeouts, cron per-job overrides). Short polling windows such as
/// `job_status.timeout_ms` or `browser.wait_for.timeout` are intentionally not
/// governed by this policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRuntimeTimeoutOverrides {
    /// Honor the model-provided value without extra audit noise.
    Allow,
    /// Honor the value but audit it. This is the default: it keeps existing
    /// capability while making accidental model-shortening visible.
    #[default]
    Warn,
    /// When the corresponding user/system runtime budget is unlimited (`0`),
    /// ignore the model's positive timeout and keep the unlimited budget.
    /// If the user configured a positive budget, the model may still tighten it.
    IgnoreWhenUserUnlimited,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeoutPolicyConfig {
    /// Runtime timeout overrides supplied by the model. Defaults to `warn` so
    /// existing behavior is preserved but visible in logs/metadata.
    #[serde(default)]
    pub model_runtime_overrides: ModelRuntimeTimeoutOverrides,
}

// ── Quick Prompt Config ──────────────────────────────────────────

pub const MAX_QUICK_PROMPT_CONTENT_CHARS: usize = 20_000;
const MAX_QUICK_PROMPT_TITLE_CHARS: usize = 80;
const MAX_QUICK_PROMPT_ITEMS: usize = 200;

/// One reusable user prompt inserted from the chat composer `#` picker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QuickPromptItem {
    pub id: String,
    pub title: String,
    pub content: String,
    pub created_at: String,
}

/// User-global quick prompts. Distinct from [`ShortcutConfig`], which controls
/// OS-level keyboard shortcuts.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QuickPromptConfig {
    #[serde(default)]
    pub items: Vec<QuickPromptItem>,
}

/// Result returned by owner-plane "add quick prompt" calls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QuickPromptAddResult {
    pub item: QuickPromptItem,
    pub duplicate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuickPromptError {
    Empty,
    TooLong { max_chars: usize },
}

impl std::fmt::Display for QuickPromptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QuickPromptError::Empty => write!(f, "quick prompt content is empty"),
            QuickPromptError::TooLong { max_chars } => {
                write!(f, "quick prompt content exceeds {} characters", max_chars)
            }
        }
    }
}

impl std::error::Error for QuickPromptError {}

impl QuickPromptConfig {
    pub fn add_prompt(&mut self, content: &str) -> Result<QuickPromptAddResult, QuickPromptError> {
        let created_at = chrono::Utc::now().to_rfc3339();
        self.add_prompt_with_created_at(content, created_at)
    }

    fn add_prompt_with_created_at(
        &mut self,
        content: &str,
        created_at: String,
    ) -> Result<QuickPromptAddResult, QuickPromptError> {
        let normalized = normalize_quick_prompt_content(content)?;
        if let Some(existing) = self.items.iter().find(|item| item.content == normalized) {
            return Ok(QuickPromptAddResult {
                item: existing.clone(),
                duplicate: true,
            });
        }

        let item = QuickPromptItem {
            id: uuid::Uuid::new_v4().to_string(),
            title: quick_prompt_title(&normalized),
            content: normalized,
            created_at,
        };
        self.items.insert(0, item.clone());
        self.items.truncate(MAX_QUICK_PROMPT_ITEMS);
        Ok(QuickPromptAddResult {
            item,
            duplicate: false,
        })
    }
}

fn normalize_quick_prompt_content(content: &str) -> Result<String, QuickPromptError> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(QuickPromptError::Empty);
    }
    if trimmed.chars().count() > MAX_QUICK_PROMPT_CONTENT_CHARS {
        return Err(QuickPromptError::TooLong {
            max_chars: MAX_QUICK_PROMPT_CONTENT_CHARS,
        });
    }
    Ok(trimmed.to_string())
}

fn quick_prompt_title(content: &str) -> String {
    let first_line = content
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(content);
    let mut title: String = first_line
        .trim()
        .chars()
        .take(MAX_QUICK_PROMPT_TITLE_CHARS)
        .collect();
    if title.is_empty() {
        title = "Quick prompt".to_string();
    }
    title
}

// ── Notification Config ─────────────────────────────────────────

/// Global notification configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationConfig {
    /// Global on/off toggle (default: true)
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
    /// Include assistant reply previews in chat-completion notifications.
    #[serde(default = "crate::default_true")]
    pub show_chat_content: bool,
    /// Fire a desktop notification when a background job (R4: tool / group)
    /// finishes — gated by [`enabled`](Self::enabled) and only when the window
    /// is in the background (`notifyIfBackground`). Default: true. The "跑完叫我"
    /// half of the Background Jobs P1 experience.
    #[serde(default = "crate::default_true")]
    pub notify_on_background_job_complete: bool,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            show_chat_content: true,
            notify_on_background_job_complete: true,
        }
    }
}

// ── Startup Notification Config ─────────────────────────────────

/// Configuration for the IM startup-notification subsystem
/// (`crates/ha-core/src/channel/worker/startup_watcher.rs`).
///
/// On every fresh process boot the watcher fans a short "back online" notice
/// out to every IM chat that had a non-incognito, non-cron, non-subagent
/// conversation within `window_secs`. Each chat is rate-limited by
/// `cooldown_secs` (per chat) and the entire fan-out is capped at
/// `global_max`. Crash loops (`HOPE_AGENT_CRASH_COUNT >=
/// crash_loop_threshold`) suppress the notice entirely.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupNotificationConfig {
    /// Master switch. When false the watcher is a no-op.
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
    /// Activity window for picking recipients (seconds). Default 72h.
    #[serde(default = "default_startup_window_secs")]
    pub window_secs: u64,
    /// Hard upper bound on the number of chats notified in one boot,
    /// across all channels and accounts. Default 30. Prevents fan-out
    /// blowup on machines with many active conversations.
    #[serde(default = "default_startup_global_max")]
    pub global_max: usize,
    /// Per-chat cooldown (seconds) — a chat that was notified more
    /// recently than this is silently skipped. Default 1800 (30 min).
    #[serde(default = "default_startup_cooldown_secs")]
    pub cooldown_secs: u64,
    /// If `HOPE_AGENT_CRASH_COUNT` env var is set to a number `>=` this
    /// threshold, the watcher suppresses the notice entirely to avoid
    /// pestering the user during a crash loop. Default 3.
    #[serde(default = "default_startup_crash_threshold")]
    pub crash_loop_threshold: u32,
}

fn default_startup_window_secs() -> u64 {
    72 * 3600
}
fn default_startup_global_max() -> usize {
    30
}
fn default_startup_cooldown_secs() -> u64 {
    30 * 60
}
fn default_startup_crash_threshold() -> u32 {
    3
}

impl Default for StartupNotificationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            window_secs: default_startup_window_secs(),
            global_max: default_startup_global_max(),
            cooldown_secs: default_startup_cooldown_secs(),
            crash_loop_threshold: default_startup_crash_threshold(),
        }
    }
}

// ── Deferred Tools Config ───────────────────────────────────────

pub const DEFAULT_DEFERRED_TOOL_NAMES: &[&str] = &[
    "acp_spawn",
    "browser",
    "get_weather",
    "image",
    "issue_report",
    "knowledge_recall",
    "list_settings_backups",
    "mac_control",
    "pdf",
    "restore_settings_backup",
    "team",
];

fn default_deferred_tools_enabled() -> bool {
    true
}

pub fn default_deferred_tool_names() -> Vec<String> {
    DEFAULT_DEFERRED_TOOL_NAMES
        .iter()
        .map(|name| (*name).to_string())
        .collect()
}

/// Configuration for deferred tool loading.
/// `enabled` turns on the mechanism; `tool_names` is the explicit set of
/// built-in tools whose schemas should be withheld from the initial LLM
/// request and discovered via `tool_search`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeferredToolsMode {
    Recommended,
    Custom,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeferredToolsConfig {
    /// V2 policy. `None` is a legacy config and is interpreted without
    /// overwriting ambiguous user choices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<DeferredToolsMode>,
    /// Enable deferred tool loading (default: true for low-frequency tools).
    #[serde(default = "default_deferred_tools_enabled")]
    pub enabled: bool,
    /// Built-in tool names explicitly deferred by the user. Defaults to the
    /// low-frequency / large-schema tools marked `default_deferred`.
    #[serde(default = "default_deferred_tool_names")]
    pub tool_names: Vec<String>,
}

impl Default for DeferredToolsConfig {
    fn default() -> Self {
        Self {
            mode: Some(DeferredToolsMode::Recommended),
            enabled: default_deferred_tools_enabled(),
            tool_names: default_deferred_tool_names(),
        }
    }
}

impl DeferredToolsConfig {
    pub fn effective_mode(&self) -> DeferredToolsMode {
        if let Some(mode) = self.mode {
            return mode;
        }
        if !self.enabled {
            return DeferredToolsMode::Disabled;
        }
        let recommended = default_deferred_tool_names();
        if self.tool_names.len() == recommended.len()
            && recommended
                .iter()
                .all(|name| self.tool_names.iter().any(|configured| configured == name))
        {
            DeferredToolsMode::Recommended
        } else {
            // Includes the historically ambiguous explicit empty list. Never
            // silently rewrite it to the current recommendation.
            DeferredToolsMode::Custom
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.effective_mode() != DeferredToolsMode::Disabled
    }
}

// ── Async Tools Config ──────────────────────────────────────────

/// Configuration for the async tool execution feature.
///
/// Async-capable tools (e.g. `exec`, `web_search`, `image_generate`) can be
/// detached into background jobs in three ways:
/// 1. The model passes `run_in_background: true` in tool args (explicit opt-in).
/// 2. The agent policy forces it (`async_tool_policy = "always-background"`).
/// 3. A sync call exceeds `auto_background_secs` (auto-transfer fallback).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsyncToolsConfig {
    /// Master switch. When false, all tool calls run synchronously regardless
    /// of `run_in_background` / agent policy.
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
    /// Auto-background budget for sync calls of async-capable tools.
    /// When a sync call exceeds this many seconds, the still-running future
    /// is transferred to a background async job and a synthetic job_id is
    /// returned to the model so the conversation can continue. The real
    /// result is delivered later via auto-injection. Default: 0 (disabled).
    /// Set a positive value to enable auto-backgrounding.
    #[serde(default = "default_async_auto_background_secs")]
    pub auto_background_secs: u64,
    /// Maximum time (seconds) a single backgrounded job *attempt* may run before
    /// being killed. Default: 0 (no async-job limit); individual tools may still
    /// enforce their own timeouts when the model sets one (for example
    /// `exec.timeout`). Per-call `job_timeout_secs` can set a timeout when this
    /// is 0 unless `timeout_policy.modelRuntimeOverrides` ignores it, or tighten
    /// the configured limit when this is positive.
    /// **R7.4 note:** this is a PER-ATTEMPT budget — a retry-eligible job that
    /// fails (not times out) and retries gets a fresh budget per attempt, so a
    /// retried job's total wall-clock can reach `max_job_secs × max_retry_attempts`
    /// plus backoffs. A timeout itself is never retried, so the budget still
    /// bounds any single execution.
    #[serde(default = "default_async_max_job_secs")]
    pub max_job_secs: u64,
    /// Number of result bytes to keep as the SQLite preview. Full completed
    /// output is spooled to `~/.hope-agent/async_jobs/<job_id>.txt`; larger
    /// previews use a head/tail shape. Default: 4096.
    #[serde(default = "default_async_inline_result_bytes")]
    pub inline_result_bytes: usize,
    /// Retention period for terminal async job rows + their spool files.
    /// Jobs whose `completed_at` is older than this are purged by a daily
    /// background task (plus one sweep at startup). Default: 30 days.
    /// `0` disables retention entirely.
    #[serde(default = "default_async_retention_secs")]
    pub retention_secs: u64,
    /// Orphan spool file grace period. Files under `~/.hope-agent/async_jobs/`
    /// whose name is not referenced by any row and whose mtime is older than
    /// this many seconds are considered orphaned and deleted. Default: 24h.
    /// The grace window prevents races with in-flight writes from freshly
    /// started jobs whose DB row hasn't committed yet.
    #[serde(default = "default_async_orphan_grace_secs")]
    pub orphan_grace_secs: u64,
    /// Legacy ceiling for hidden `job_status(block=true)` waits. The
    /// model-facing `job_status` schema is snapshot-only, and the tool applies
    /// an additional short UI-safety cap so status polling cannot block a chat
    /// turn for minutes. Used only when `max_job_secs == 0`; otherwise the
    /// runtime ceiling still mirrors `max_job_secs`. Default: 7200 (2h).
    #[serde(default = "default_async_job_status_max_wait_secs")]
    pub job_status_max_wait_secs: u64,
    /// Maximum number of explicitly-backgrounded tool jobs
    /// (`run_in_background: true` / `always-background` policy) that may run
    /// concurrently. Each background job holds a dedicated OS thread + runtime,
    /// so an uncapped model could linearly exhaust threads/memory by firing
    /// many `run_in_background` calls across rounds. When the cap is reached a
    /// new background request returns an error result telling the model to wait
    /// (it can poll `job_status`) or run synchronously. Default: hardware-derived
    /// `clamp(logical_cores - 2, 4, 16)` (see `default_async_max_concurrent_jobs`).
    /// Set to 0 for no limit. (Auto-background transfers of merely-slow sync calls
    /// are bounded separately by per-turn tool concurrency + the sync budget.)
    #[serde(default = "default_async_max_concurrent_jobs")]
    pub max_concurrent_jobs: usize,
    /// Per-session share of the background-job pool (R7.1 fairness tier). A
    /// single session (or IM chat) may hold at most this many concurrent tool
    /// jobs; further `run_in_background` calls from the same session QUEUE even
    /// when the global pool (`max_concurrent_jobs`) still has room, so one busy
    /// session can't monopolize every slot and starve the others. The scheduler
    /// promotes queued jobs per-session round-robin, skipping any session already
    /// at this cap. Auto-backgrounded jobs are *counted* against it too (but an
    /// already-running job that auto-detaches is counted, not refused — it can
    /// briefly exceed this cap). Default: hardware-derived (~3/4 of the global
    /// cap, always below it, band [3,12]). Set to 0 for no per-session limit
    /// (only the global cap applies).
    #[serde(default = "default_async_max_concurrent_jobs_per_session")]
    pub max_concurrent_jobs_per_session: usize,
    /// R7.4 retry: auto-retry a *backgrounded* job that fails with a transient
    /// error, with exponential backoff. **Opt-in (default `false`).** Only
    /// idempotent, re-runnable tools (`web_search` / `web_fetch`) are ever
    /// retried; `exec` (could repeat a half-applied side effect) and
    /// `image_generate` (re-runs to a different, re-billed image) are NEVER
    /// retried regardless of this switch (eligibility is a code-level allowlist,
    /// not a knob). User cancels / policy denials / timeouts are never retried.
    /// Default is off because an eligible tool still re-RUNS — and `web_search`
    /// is often a *paid* provider, so retrying a deterministic failure (e.g. a
    /// 400 bad query) would re-bill; the user opts in to that trade-off. Per-tool
    /// retry-eligibility is in `async_jobs::retry::is_retry_eligible`.
    #[serde(default)]
    pub retry_enabled: bool,
    /// R7.4: total attempts for a retry-eligible backgrounded job (1 = no retry;
    /// the initial try counts), hard-capped at 10. Backoff between attempts is
    /// exponential from a fixed 500ms base. Default: 3.
    #[serde(default = "default_async_max_retry_attempts")]
    pub max_retry_attempts: u32,
    /// Completion-injection merge window (R4), in seconds. When multiple
    /// background jobs in the SAME session finish within this window, their
    /// completion notifications are merged into ONE injected turn (one
    /// `<task-notification-batch>` listing every task) instead of N separately
    /// billed turns — so "encourage backgrounding" doesn't degrade into "spam
    /// billed turns". The first completion opens the window; everything settling
    /// before it elapses joins the batch. A `Group` (R5) is the pre-merged
    /// special case and bypasses this. Default: 3. Set to 0 to disable merging
    /// (each job injects immediately).
    #[serde(default = "default_async_completion_merge_window_secs")]
    pub completion_merge_window_secs: u64,
    /// R9: bytes of *running* output retained per backgrounded `exec` job (R3 ①
    /// tail ring). While a backgrounded `exec` runs, its stdout/stderr is teed
    /// into a per-job ring of this size so `job_status(action:status)` can show
    /// the latest output (judge "still working" vs "stuck") without waiting for
    /// completion. Larger = more visibility, more RAM per running job (the ring
    /// is bounded by the concurrent-job cap). The cap is snapshotted when the
    /// job starts; changing this does not resize an already-running job's ring.
    /// Default: 8192. Clamped at read to `[256, 1048576]` (256B–1MB).
    #[serde(default = "default_async_output_tail_bytes")]
    pub output_tail_bytes: usize,
    /// R9: hard ceiling on the in-memory background-job wait queue (R7.1). When
    /// every slot (`max_concurrent_jobs` / `max_concurrent_jobs_per_session`) is
    /// full, further `run_in_background` requests QUEUE here; each queued job
    /// pins its live `ToolExecContext` in RAM, so the queue MUST stay bounded —
    /// past this a new background request hard-rejects (the model waits / runs
    /// synchronously). This is a memory guardrail, not an "unlimited" knob: it is
    /// clamped at read to `[1, 4096]` (0 does NOT mean unlimited). Default: 256.
    #[serde(default = "default_async_max_queued_jobs")]
    pub max_queued_jobs: usize,
    /// R9: upper bound (seconds) on `schedule_wakeup`'s self-scheduled delay.
    /// A requested delay is clamped to `[10, wakeup_max_delay_secs]` (the 10s
    /// floor is a non-configurable busy-poll guard). Guards against zombie timers
    /// pinning a session indefinitely; longer cadences belong to cron. Clamped at
    /// read to `[10, 604800]` (10s–7d). Default: 86400 (24h).
    #[serde(default = "default_wakeup_max_delay_secs")]
    pub wakeup_max_delay_secs: u64,
    /// R9: per-session cap on pending `schedule_wakeup` wakeups. Exceeding it is a
    /// structural reject (it does NOT queue) — guards against an agent
    /// self-scheduling a flood of billed turns. Clamped at read to `[1, 100]`.
    /// Default: 5.
    #[serde(default = "default_wakeup_max_pending_per_session")]
    pub wakeup_max_pending_per_session: usize,
}

fn default_async_auto_background_secs() -> u64 {
    0
}
fn default_async_max_job_secs() -> u64 {
    0
}
fn default_async_inline_result_bytes() -> usize {
    4096
}
fn default_async_retention_secs() -> u64 {
    30 * crate::SECS_PER_DAY
}
fn default_async_orphan_grace_secs() -> u64 {
    24 * crate::SECS_PER_HOUR
}
fn default_async_job_status_max_wait_secs() -> u64 {
    7200
}
fn default_async_completion_merge_window_secs() -> u64 {
    3
}
fn default_async_output_tail_bytes() -> usize {
    8 * 1024
}
fn default_async_max_queued_jobs() -> usize {
    256
}
fn default_wakeup_max_delay_secs() -> u64 {
    86_400
}
fn default_wakeup_max_pending_per_session() -> usize {
    5
}
fn default_async_max_retry_attempts() -> u32 {
    // 3 total attempts (1 initial + 2 retries) for retry-eligible tools. A
    // user-set 1 disables retry for everything; `retry_enabled = false` is the
    // master kill-switch.
    3
}
fn default_async_max_concurrent_jobs_per_session() -> usize {
    // Per-session fairness share (R7.1), **derived from the global default** so it
    // ALWAYS leaves headroom for other sessions on every hardware tier. A fixed
    // value can't: the global cap is itself hardware-derived (`clamp(cores-2,4,16)`,
    // band [4,16]), so a fixed `6` would be >= the global cap on common ≤8-logical-
    // core machines (8-thread laptop → global 6) and silently no-op — a single
    // session fills the whole global pool before its per-session cap ever bites.
    // ~3/4 of the global cap (band [3,12], always strictly below it) lets one
    // focused session use most of the pool while still reserving slots for others.
    // A user-set `0` means no per-session limit (handled in the slot acquire path).
    (default_async_max_concurrent_jobs() * 3 / 4).max(2)
}
fn default_async_max_concurrent_jobs() -> usize {
    // Hardware-derived default so the cap doesn't oversubscribe the machine:
    // `clamp(logical_cores - 2, 4, 16)`. `available_parallelism` reports logical
    // cores (incl. SMT); we leave 2 for the main loop + UI/IO. A user-set `0`
    // still means unlimited (handled in the slot acquire path, not here).
    // Aligned with the Workflow engine's `min(16, cores - 2)` concurrency cap.
    std::thread::available_parallelism()
        .map(|n| n.get().saturating_sub(2).clamp(4, 16))
        .unwrap_or(8)
}

impl AsyncToolsConfig {
    /// Runtime upper bound on a single hidden `job_status(block=true)` wait,
    /// in seconds. The tool may apply a smaller UI-safety cap.
    /// Mirrors `max_job_secs` when it's positive; otherwise falls back to
    /// `job_status_max_wait_secs` (clamped to ≥ 1).
    pub fn job_status_ceiling_secs(&self) -> u64 {
        if self.max_job_secs == 0 {
            self.job_status_max_wait_secs.max(1)
        } else {
            self.max_job_secs
        }
    }
}

impl Default for AsyncToolsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_background_secs: default_async_auto_background_secs(),
            max_job_secs: default_async_max_job_secs(),
            inline_result_bytes: default_async_inline_result_bytes(),
            retention_secs: default_async_retention_secs(),
            orphan_grace_secs: default_async_orphan_grace_secs(),
            job_status_max_wait_secs: default_async_job_status_max_wait_secs(),
            max_concurrent_jobs: default_async_max_concurrent_jobs(),
            max_concurrent_jobs_per_session: default_async_max_concurrent_jobs_per_session(),
            retry_enabled: false,
            max_retry_attempts: default_async_max_retry_attempts(),
            completion_merge_window_secs: default_async_completion_merge_window_secs(),
            output_tail_bytes: default_async_output_tail_bytes(),
            max_queued_jobs: default_async_max_queued_jobs(),
            wakeup_max_delay_secs: default_wakeup_max_delay_secs(),
            wakeup_max_pending_per_session: default_wakeup_max_pending_per_session(),
        }
    }
}

/// Cron (scheduled task) subsystem configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CronConfig {
    /// Maximum number of cron jobs that may execute concurrently. Each cron run
    /// is a full agent turn (it can spawn sub-agents / tools), so a herd of jobs
    /// all due at the same instant could otherwise spawn dozens of simultaneous
    /// LLM turns and exhaust the machine / trip provider rate limits. The
    /// scheduler acquires a slot **before** claiming a due job, so a job it
    /// can't run yet keeps its `next_run_at` and is retried next tick instead of
    /// silently skipping the occurrence (slot-before-claim). Manual `run now`
    /// bypasses the cap but its running marker still counts toward occupancy, so
    /// the scheduler won't over-spawn while a manual run is in flight.
    /// Default: 5. `0` = unlimited.
    #[serde(default = "default_cron_max_concurrent")]
    pub max_concurrent: u32,

    /// Per-run wall-clock timeout in seconds. `0` = no cron-level timeout.
    /// Positive values are clamped to `[30, 7200]` (30s–2h). A per-job
    /// `CronJob.job_timeout_secs` override takes precedence when set; this is
    /// the global fallback. Default: 0 (no cron-level timeout).
    #[serde(default = "default_cron_job_timeout_secs")]
    pub job_timeout_secs: u64,

    /// Grace window (seconds) for late-firing a one-shot `At` job that came due
    /// while the app was down. On startup an `At` job past its scheduled time by
    /// **no more than** this window still fires (catch-up); one past it by more is
    /// marked `missed`. `0` = strict (any past-due `At` is missed — the pre-§7
    /// behavior); capped at read to 7 days. Default: 300 (5min) so a brief restart
    /// doesn't silently drop a scheduled one-shot. (A claimed-then-crashed `At` —
    /// `next_run_at` already cleared — is always marked `missed`, never re-fired,
    /// regardless of this window, since it may have partially executed.)
    #[serde(default = "default_cron_at_grace_secs")]
    pub at_grace_secs: u64,
}

impl Default for CronConfig {
    fn default() -> Self {
        Self {
            max_concurrent: default_cron_max_concurrent(),
            job_timeout_secs: default_cron_job_timeout_secs(),
            at_grace_secs: default_cron_at_grace_secs(),
        }
    }
}

impl CronConfig {
    /// Effective concurrency limit. `None` means unlimited (`max_concurrent == 0`).
    pub fn effective_max_concurrent(&self) -> Option<usize> {
        match self.max_concurrent {
            0 => None,
            n => Some(n as usize),
        }
    }

    /// Per-run timeout, clamped to the safe band `[30, 7200]` seconds for
    /// positive values. `0` means no cron-level timeout.
    pub fn effective_job_timeout_secs(&self) -> u64 {
        clamp_cron_job_timeout_secs(self.job_timeout_secs)
    }

    /// Late-fire grace window in seconds, capped at 7 days. Unlike the timeout,
    /// `0` is preserved (it means "strict — no late-fire"), so only the upper
    /// bound is clamped.
    pub fn effective_at_grace_secs(&self) -> u64 {
        self.at_grace_secs.min(604_800)
    }
}

/// Clamp a per-run cron timeout — the global `CronConfig.job_timeout_secs` or a
/// per-job `CronJob.job_timeout_secs` override. `0` means no cron-level timeout;
/// positive values are clamped to `[30, 7200]` seconds.
pub fn clamp_cron_job_timeout_secs(secs: u64) -> u64 {
    if secs == 0 {
        0
    } else {
        secs.clamp(30, 7200)
    }
}

pub use crate::permission::ApprovalTimeoutAction;
pub use crate::permission::UnattendedApprovalAction;

// ── Default helpers ─────────────────────────────────────────────

fn default_cron_max_concurrent() -> u32 {
    5
}

fn default_cron_job_timeout_secs() -> u64 {
    0
}

fn default_cron_at_grace_secs() -> u64 {
    300
}

fn default_skill_env_check() -> bool {
    true
}

fn default_conditional_skills_enabled() -> bool {
    true
}

fn default_tool_call_narration_enabled() -> bool {
    true
}

fn default_default_agent_id() -> Option<String> {
    Some(crate::agent_loader::DEFAULT_AGENT_ID.to_string())
}

fn default_reasoning_effort() -> String {
    "medium".to_string()
}

pub(crate) fn default_tool_timeout() -> u64 {
    0
}

pub(crate) fn default_ask_user_question_timeout() -> u64 {
    0
}

pub(crate) fn default_theme() -> String {
    "auto".to_string()
}

pub(crate) fn default_language() -> String {
    "auto".to_string()
}

pub const SIDEBAR_UI_MODE_COMPACT: &str = "compact";
pub const SIDEBAR_UI_MODE_DETAILED: &str = "detailed";

pub fn default_sidebar_ui_mode() -> String {
    SIDEBAR_UI_MODE_DETAILED.to_string()
}

pub fn normalize_sidebar_ui_mode(mode: &str) -> String {
    match mode {
        SIDEBAR_UI_MODE_COMPACT => SIDEBAR_UI_MODE_COMPACT.to_string(),
        SIDEBAR_UI_MODE_DETAILED => SIDEBAR_UI_MODE_DETAILED.to_string(),
        _ => SIDEBAR_UI_MODE_DETAILED.to_string(),
    }
}

// ── Recap Config ────────────────────────────────────────────────

fn default_recap_default_range_days() -> u32 {
    30
}
fn default_recap_max_sessions_per_report() -> u32 {
    500
}
fn default_recap_facet_concurrency() -> u8 {
    4
}
fn default_recap_cache_retention_days() -> u32 {
    180
}

/// Configuration for the `/recap` deep-analysis report feature.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecapConfig {
    /// Deprecated — superseded by `model_override`. Agent ID used to extract
    /// per-session facets and generate report sections. Kept for backward
    /// compatibility: still consulted (resolved to an equivalent
    /// `ModelChain` via the agent's own model config) when `model_override`
    /// is unset, so existing configurations keep working, but the GUI no
    /// longer writes this field.
    #[serde(default)]
    pub analysis_agent: Option<String>,
    /// Model chain override for facet extraction and report section
    /// generation. `None` = fall through to `function_models.automation`
    /// (or the deprecated `analysis_agent`, if still set) → chat default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<crate::provider::ModelChain>,
    /// Output language for generated reports. `None` or "auto" follows the
    /// global UI language (`AppConfig.language`, which itself may be "auto" →
    /// system locale). A specific locale code (e.g. "zh", "en") overrides it.
    #[serde(default)]
    pub language: Option<String>,
    /// Default time window (days) when no prior report exists.
    #[serde(default = "default_recap_default_range_days")]
    pub default_range_days: u32,
    /// Hard cap on number of sessions analyzed in a single report.
    #[serde(default = "default_recap_max_sessions_per_report")]
    pub max_sessions_per_report: u32,
    /// Concurrency for per-session facet extraction.
    #[serde(default = "default_recap_facet_concurrency")]
    pub facet_concurrency: u8,
    /// Days to retain cached session facets before garbage collection.
    #[serde(default = "default_recap_cache_retention_days")]
    pub cache_retention_days: u32,
}

impl Default for RecapConfig {
    fn default() -> Self {
        Self {
            analysis_agent: None,
            model_override: None,
            language: None,
            default_range_days: default_recap_default_range_days(),
            max_sessions_per_report: default_recap_max_sessions_per_report(),
            facet_concurrency: default_recap_facet_concurrency(),
            cache_retention_days: default_recap_cache_retention_days(),
        }
    }
}

// ── Embedded Server Config ──────────────────────────────────────

fn default_server_bind() -> String {
    "127.0.0.1:8420".to_string()
}

/// Embedded HTTP/WS server configuration, stored in config.json `server` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedServerConfig {
    /// Bind address (default "127.0.0.1:8420").
    /// Set to "0.0.0.0:8420" to expose to the network.
    #[serde(default = "default_server_bind")]
    pub bind_addr: String,
    /// API Key for authenticating requests (None = no auth).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Token limited to read-only `/api/knowledge/agent/{search,read,expand,sources}`.
    /// The global `api_key` remains the owner token for every API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_agent_read_token: Option<String>,
    /// Publicly-reachable base URL for this server, used when IM channels
    /// that only accept remote HTTPS media (LINE / QQ Bot native media, IRC
    /// text fallback) need to send `/api/attachments/...` links. `None`
    /// disables those fallbacks.
    /// Format: `https://example.com` (no trailing slash).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_base_url: Option<String>,
}

impl EmbeddedServerConfig {
    /// Merge a partial update over an existing server config.
    ///
    /// This matches onboarding semantics for optional secrets:
    /// - `None` keeps the existing value (the GUI only receives masked values).
    /// - `Some("")` clears the value.
    /// - `Some(value)` replaces it.
    pub fn merge_over_existing(mut self, existing: &EmbeddedServerConfig) -> Self {
        if self.bind_addr.trim().is_empty() {
            self.bind_addr = existing.bind_addr.clone();
        }
        self.api_key = merge_optional_config_string(self.api_key, existing.api_key.clone());
        self.knowledge_agent_read_token = merge_optional_config_string(
            self.knowledge_agent_read_token,
            existing.knowledge_agent_read_token.clone(),
        );
        self.public_base_url =
            merge_optional_config_string(self.public_base_url, existing.public_base_url.clone());
        self
    }
}

fn merge_optional_config_string(next: Option<String>, existing: Option<String>) -> Option<String> {
    match next {
        Some(value) if value.is_empty() => None,
        Some(value) => Some(value),
        None => existing,
    }
}

impl Default for EmbeddedServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: default_server_bind(),
            api_key: None,
            knowledge_agent_read_token: None,
            public_base_url: None,
        }
    }
}

// ── Onboarding State ────────────────────────────────────────────

/// Current onboarding wizard version. Bump this only when existing users
/// must re-walk the flow after meaningful required additions. Optional
/// steps that should appear only for new installs and manual reruns keep
/// this value unchanged.
pub const CURRENT_ONBOARDING_VERSION: u32 = 1;

/// First-run onboarding wizard state. Drives both the GUI wizard
/// (`src/components/onboarding`) and the CLI wizard
/// (`src-tauri/src/cli_onboarding`).
///
/// A user is considered "onboarded" when `completed_version >=
/// CURRENT_ONBOARDING_VERSION`. `0` (the default) means never completed, so
/// new installs land in the wizard. When a user quits mid-wizard, the
/// front-end writes `draft` + `draft_step` so the next launch can resume.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingState {
    /// Highest wizard version the user has finished. `0` = never completed.
    #[serde(default)]
    pub completed_version: u32,
    /// ISO 8601 timestamp of the most recent completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    /// Step keys the user explicitly skipped in the most recent run.
    #[serde(default)]
    pub skipped_steps: Vec<String>,
    /// Partially-filled draft captured when the user exits mid-wizard.
    /// The shape is an opaque JSON object owned by the front-end; the
    /// backend only persists and returns it verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<serde_json::Value>,
    /// Step index (0-based) the user was on when they exited.
    #[serde(default)]
    pub draft_step: u32,
    /// Sticky flag: true once the wizard has ever completed at any version.
    /// Survives `reset()` so explicit rerun doesn't get caught by the
    /// legacy-upgrade heuristic in `infer_legacy_completed`, which would
    /// otherwise skip the wizard for users who already have providers.
    #[serde(default)]
    pub ever_completed: bool,
}

/// Filesystem / file-browser policy.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FilesystemConfig {
    /// Allow file-browser **write** operations (create / delete / rename /
    /// mkdir / upload) over the HTTP transport. Default `false`: remote HTTP
    /// clients get read-only browsing, while the desktop (Tauri IPC) always
    /// writes. HIGH-risk: enabling lets any token-bearing client modify files
    /// in the project working directory on the server host.
    #[serde(default)]
    pub allow_remote_writes: bool,
}

// ── App Config ──────────────────────────────────────────────────

/// Root structure for the application's persisted configuration (`config.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub active_model: Option<ActiveModel>,
    /// Global fallback model chain (ordered).
    /// When the primary model fails, these are tried in order.
    #[serde(default)]
    pub fallback_models: Vec<ActiveModel>,
    /// Global default agent id used when neither the explicit caller nor a
    /// session/project/channel specifies one. Defaults to `"ha-main"`.
    #[serde(default = "default_default_agent_id")]
    pub default_agent_id: Option<String>,
    /// User-defined display order for agent pickers and sidebar lists. Missing
    /// or newly-created agents fall back to the default main-first ordering.
    #[serde(default)]
    pub agent_order: Vec<String>,
    /// Extra directories to scan for skills
    #[serde(default)]
    pub extra_skills_dirs: Vec<String>,
    /// Disabled skill names
    #[serde(default)]
    pub disabled_skills: Vec<String>,
    /// Whether to check skill runtime requirements before injecting. Default
    /// true. Hard blockers (currently unsupported OS) hide the skill; missing
    /// installable/configurable dependencies remain visible and are diagnosed
    /// at activation time. When false, all skills are injected regardless of
    /// environment.
    #[serde(default = "default_skill_env_check")]
    pub skill_env_check: bool,
    /// Kill switch for `paths:` conditional skill activation. Default true.
    /// When false, file/path-aware tools stop activating `paths:` skills, so
    /// those skills remain hidden unless they have already been activated for
    /// the session.
    #[serde(default = "default_conditional_skills_enabled")]
    pub conditional_skills_enabled: bool,
    /// Reusable embedding model configurations.
    #[serde(default)]
    pub embedding_models: Vec<crate::memory::EmbeddingModelConfig>,
    /// Active memory vector-search embedding selection.
    #[serde(default)]
    pub memory_embedding: crate::memory::EmbeddingSelection,
    /// Active knowledge-base vector-search embedding selection. Independent of
    /// `memory_embedding` (knowledge has its own enable / model / signature /
    /// reembed lifecycle) but draws from the same shared `embedding_models`
    /// library — see D7.
    #[serde(default)]
    pub knowledge_embedding: crate::memory::EmbeddingSelection,
    /// Knowledge note chunking parameters (D12, advanced). Changing them
    /// triggers a full reindex (re-chunk + re-embed) of every KB. GUI-only
    /// (dedicated owner commands, not `update_settings`) like `knowledge_embedding`.
    #[serde(default)]
    pub knowledge_chunk: crate::knowledge::ChunkConfig,
    /// Knowledge hybrid `note_search` ranking parameters (fusion weights / RRF-k /
    /// MMR-λ / candidate pool). Pure query-time, no reindex — a normal MEDIUM
    /// setting (GUI + `update_settings`), unlike `knowledge_chunk`.
    #[serde(default)]
    pub knowledge_search: crate::knowledge::KnowledgeSearchConfig,
    /// Knowledge source-to-note organization agent. Defaults to inheriting the
    /// global default agent, but can be pinned independently from recap/chat.
    #[serde(default)]
    pub knowledge_compile: crate::knowledge::KnowledgeCompileConfig,
    /// Knowledge Layer-2 autonomous maintenance (WS6): scheduling + per-task
    /// toggles + auto-approve for the proposal review queue. Disabled by default.
    #[serde(default)]
    pub knowledge_maintenance: crate::knowledge::maintenance::MaintenanceConfig,
    /// Model selection for Knowledge's vision-capable ingestion (image OCR
    /// import); see `crate::automation::run_vision`.
    #[serde(default)]
    pub knowledge_vision: crate::knowledge::KnowledgeVisionConfig,
    /// Model selection for the standalone note-authoring tools (note_distill /
    /// note_moc / session_to_note).
    #[serde(default)]
    pub note_tools: crate::knowledge::NoteToolsConfig,
    /// Read bridge ③ — passive related-notes prompt (Phase 3, D7): each user turn
    /// surfaces the top accessible-KB note titles as an independent untrusted
    /// cache block. Enabled by default because it is retrieval-only, title-only,
    /// and still fully gated by session/IM/incognito KB access. MEDIUM risk
    /// (context/cost), writable via `update_settings`.
    #[serde(default)]
    pub knowledge_passive_recall: crate::knowledge::PassiveRecallConfig,
    /// Optional retention for original audio/video/image source files and image
    /// thumbnails. HIGH/privacy setting; disabled by default and controlled by
    /// owner-plane settings only.
    #[serde(default)]
    pub knowledge_media_retention: crate::knowledge::KnowledgeMediaRetentionConfig,
    /// Deprecated legacy embedding config. Kept as a deserialization sink only;
    /// user-facing embedding config lives in `embedding_models` +
    /// `memory_embedding`.
    #[serde(default, skip_serializing)]
    pub embedding: crate::memory::EmbeddingConfig,
    /// Web search provider configuration
    #[serde(default)]
    pub web_search: crate::tools::web_search::WebSearchConfig,
    /// Web fetch tool configuration
    #[serde(default)]
    pub web_fetch: crate::tools::web_fetch::WebFetchConfig,
    /// SSRF policy configuration (browser / web_fetch / image_generate / url_preview)
    #[serde(default)]
    pub ssrf: crate::security::ssrf::SsrfConfig,
    /// Filesystem / file-browser policy (HTTP remote-write gate).
    #[serde(default)]
    pub filesystem: FilesystemConfig,
    /// Per-skill environment variable overrides configured by user.
    /// Outer key: skill name, inner key: env var name, value: env var value.
    #[serde(default)]
    pub skill_env: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    /// Global memory auto-extract configuration
    #[serde(default)]
    pub memory_extract: crate::memory::MemoryExtractConfig,
    /// Memory UX v2 product-level contract. Legacy fields remain live during
    /// the staged rollout; this config is shadow-only until rollout.enabled.
    #[serde(default)]
    pub memory: crate::memory::MemoryRuntimeConfig,
    /// LLM-based memory selection configuration
    #[serde(default)]
    pub memory_selection: crate::memory::MemorySelectionConfig,
    /// Per-section character budgets for the system-prompt Memory block.
    #[serde(default)]
    pub memory_budget: crate::memory::MemoryBudgetConfig,
    /// Memory deduplication thresholds
    #[serde(default)]
    pub dedup: crate::memory::DedupConfig,
    /// Additive external memory providers. Default disabled; local memory
    /// remains the source of truth and agent-plane recall/write paths ignore
    /// this until a concrete owner-approved adapter is wired.
    #[serde(default)]
    pub memory_providers: crate::memory::ExternalMemoryProvidersConfig,
    /// Hybrid search weight configuration
    #[serde(default)]
    pub hybrid_search: crate::memory::HybridSearchConfig,
    /// Temporal decay configuration for memory search
    #[serde(default)]
    pub temporal_decay: crate::memory::TemporalDecayConfig,
    /// MMR reranking configuration
    #[serde(default)]
    pub mmr: crate::memory::MmrConfig,
    /// Multimodal embedding configuration (image/audio)
    #[serde(default)]
    pub multimodal: crate::memory::MultimodalConfig,
    /// Embedding cache configuration
    #[serde(default)]
    pub embedding_cache: crate::memory::EmbeddingCacheConfig,
    /// Context compaction configuration
    #[serde(default)]
    pub compact: crate::context_compact::CompactConfig,
    /// LLM-generated session title configuration
    #[serde(default)]
    pub session_title: crate::session_title::SessionTitleConfig,
    /// Notification configuration
    #[serde(default)]
    pub notification: NotificationConfig,
    /// IM startup-notification configuration. Drives the short "back
    /// online" notice sent on every fresh boot to chats active within
    /// `startup_notification.window_secs`. See
    /// `channel::worker::startup_watcher`.
    #[serde(default)]
    pub startup_notification: StartupNotificationConfig,
    /// Image generation configuration
    #[serde(default)]
    pub image_generate: crate::tools::image_generate::ImageGenConfig,
    /// Audio generation configuration (design space `audio` artifact form).
    #[serde(default)]
    pub audio_generate: crate::tools::audio_generate::AudioGenConfig,
    /// GitHub issue reporting target and defaults. Token lives separately under
    /// `~/.hope-agent/credentials/github-issue.json`.
    #[serde(default)]
    pub issue_reporting: crate::issue_reporting::IssueReportingConfig,
    /// Canvas tool configuration
    #[serde(default)]
    pub canvas: crate::tools::canvas::CanvasConfig,
    /// Design Space subsystem configuration
    #[serde(default)]
    pub design: crate::design::DesignConfig,
    /// Browser automation configuration (backend selection, default mode,
    /// user-attach profile bookkeeping).
    #[serde(default)]
    pub browser: Option<crate::browser::BrowserConfig>,
    /// Image tool configuration (max images per call, etc.)
    #[serde(default)]
    pub image: crate::tools::image::ImageToolConfig,
    /// PDF tool configuration (max PDFs, max vision pages, etc.)
    #[serde(default)]
    pub pdf: crate::tools::pdf::PdfToolConfig,
    /// Global hard timeout (seconds) for a foreground/synchronous tool execution.
    /// Safety net for when inner tool timeouts don't fire (network issues, etc.).
    /// Async background jobs bypass this and use `async_tools.max_job_secs`.
    /// Default 0 (disabled). Set a positive value to enable.
    #[serde(default = "default_tool_timeout")]
    pub tool_timeout: u64,
    /// Policy for runtime timeout values supplied by the model. This only
    /// governs timeout arguments that can shorten/kill long-running work; short
    /// waits and network request timeouts keep their own bounded semantics.
    #[serde(default)]
    pub timeout_policy: TimeoutPolicyConfig,
    /// Threshold (bytes) for persisting large tool results to disk.
    /// Results exceeding this size are written to disk with a preview in context.
    /// Default: 50000 (50KB). Set to 0 to disable.
    #[serde(default)]
    pub tool_result_disk_threshold: Option<usize>,
    /// UI theme preference: "auto" | "light" | "dark"
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Always show a stronger focus indicator, including for pointer focus.
    /// System high-contrast modes enhance focus independently of this toggle.
    #[serde(default)]
    pub enhanced_focus_indicators: bool,
    /// UI language preference: "auto" means follow system, otherwise a locale code like "zh", "en"
    #[serde(default = "default_language")]
    pub language: String,
    /// Whether UI background effects (stars, weather) are enabled. Default off.
    #[serde(default)]
    pub ui_effects_enabled: bool,
    /// Prevent the host from idle-sleeping while the app runs (user setting,
    /// default off). When on, the primary process holds an OS sleep assertion
    /// (macOS `caffeinate -i` / Linux logind inhibitor / Windows
    /// `ES_SYSTEM_REQUIRED`); the display may still turn off independently.
    #[serde(default)]
    pub prevent_sleep: bool,
    /// Sidebar visual density: "compact" (default) | "detailed"
    #[serde(default = "default_sidebar_ui_mode")]
    pub sidebar_ui_mode: String,
    /// Global proxy configuration for all outgoing HTTP requests
    #[serde(default)]
    pub proxy: ProxyConfig,
    /// Configurable limits for skill prompt generation
    #[serde(default)]
    pub skill_prompt_budget: crate::skills::SkillPromptBudget,
    /// Bundled skills allowlist (empty = all allowed)
    #[serde(default)]
    pub skill_allow_bundled: Vec<String>,

    /// ACP control plane configuration (external agent management)
    #[serde(default)]
    pub acp_control: crate::acp_control::AcpControlConfig,

    /// Global keyboard shortcut configuration
    #[serde(default)]
    pub shortcuts: ShortcutConfig,

    /// User-global reusable chat prompts inserted from the composer `#` picker.
    #[serde(default)]
    pub quick_prompts: QuickPromptConfig,

    /// Custom plans directory override. When set, plans are stored here instead of
    /// the default `~/.hope-agent/plans/`.
    #[serde(default)]
    pub plans_directory: Option<String>,

    /// Global default temperature for LLM API calls (0.0–2.0).
    /// Can be overridden at the agent level or session level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    /// Global default Think / reasoning effort. Agent and Session values may
    /// override this, but Session-bound turns never mutate it.
    #[serde(default = "default_reasoning_effort")]
    pub reasoning_effort: String,

    /// Whether to use a dedicated sub-agent for plan creation (Planning phase).
    /// When true, planning runs in an isolated sub-agent (saves main agent context).
    /// When false, planning runs inline in the main agent (preserves context continuity).
    /// Default: false (inline mode)
    #[serde(default)]
    pub plan_subagent: bool,

    /// Whether ask_user_question waits automatically expire.
    #[serde(default)]
    pub ask_user_question_timeout_enabled: bool,
    /// Timeout in seconds for ask_user_question tool waiting for user response
    /// when `ask_user_question_timeout_enabled` is true.
    /// Default duration: 0 (no timeout; wait forever).
    #[serde(default = "default_ask_user_question_timeout")]
    pub ask_user_question_timeout_secs: u64,

    /// IM channel configuration (Telegram, Discord, Slack, etc.)
    #[serde(default)]
    pub channels: crate::channel::ChannelStoreConfig,

    /// Deferred tool loading configuration
    #[serde(default)]
    pub deferred_tools: DeferredToolsConfig,

    /// Embedded HTTP/WS server configuration
    #[serde(default)]
    pub server: EmbeddedServerConfig,

    /// Recap (deep session analysis) configuration
    #[serde(default)]
    pub recap: RecapConfig,

    /// Async tool execution configuration (run_in_background, auto-background, etc.)
    #[serde(default)]
    pub async_tools: AsyncToolsConfig,

    /// Cron (scheduled task) subsystem configuration (concurrency cap, etc.)
    #[serde(default)]
    pub cron: CronConfig,

    /// Behavior awareness configuration. Provides each chat with a
    /// dynamically-refreshed view of what the user is doing in other
    /// parallel sessions.
    #[serde(default)]
    pub awareness: crate::awareness::AwarenessConfig,

    /// Knowledge-space "sprite / inspiration mode": a proactive, transient
    /// writing companion that reacts to the note being edited. Disabled by
    /// default (makes proactive LLM calls).
    #[serde(default)]
    pub sprite: crate::sprite::SpriteConfig,

    /// Offline memory consolidation ("Dreaming", Phase B3).
    /// Controls when cycles run (idle / cron / manual) and how aggressively
    /// they promote candidates into pinned core memory.
    #[serde(default)]
    pub dreaming: crate::memory::dreaming::DreamingConfig,

    /// Skills automation (Phase B'). Nests `autoReview` knobs that drive the
    /// post-conversation skill CRUD pipeline.
    #[serde(default)]
    pub skills: crate::skills::SkillsConfig,

    /// Opt-in LLM summarization layer on top of `recall_memory` /
    /// `session_search` tool output (Phase B'3). Default disabled.
    #[serde(default)]
    pub recall_summary: crate::memory::RecallSummaryConfig,

    /// Whether to inject `TOOL_CALL_NARRATION_GUIDANCE` into the system prompt.
    /// When enabled, the model announces each tool call with a one-sentence
    /// preamble (Claude Code style). Some models (e.g. GPT-5.4 via Codex) over-
    /// apply the rule and restate the same intent across consecutive tool calls,
    /// which reads as noise — disable per-user if so. Default `true` because
    /// the per-step narration also drives IM Channel UX (no tool-call UI there;
    /// silent loops feel like the bot died) and gives desktop users a live
    /// progress signal during long tool chains.
    #[serde(default = "default_tool_call_narration_enabled")]
    pub tool_call_narration_enabled: bool,

    /// Permission / approval system. Replaces the legacy top-level fields
    /// `approval_timeout_secs` / `approval_timeout_action` /
    /// `dangerous_skip_all_approvals`. See
    /// [`crate::permission::PermissionGlobalConfig`] for fields.
    #[serde(default)]
    pub permission: crate::permission::PermissionGlobalConfig,

    /// First-run onboarding wizard state. See [`OnboardingState`].
    #[serde(default)]
    pub onboarding: OnboardingState,

    /// Configured Model Context Protocol (MCP) servers. Each entry describes
    /// a stdio / http / sse / ws endpoint that contributes tools (and later
    /// prompts / resources) to the main conversation catalog. See
    /// `docs/architecture/mcp.md` for the full subsystem overview.
    #[serde(default)]
    pub mcp_servers: Vec<crate::mcp::McpServerConfig>,

    /// Global knobs shared by every MCP server: master switch, concurrency
    /// caps, backoff policy, always-load whitelist, denylist. Defaults are
    /// tuned to be safe — the MCP subsystem is `enabled=true` but
    /// `mcp_servers=[]` means new installs see no behavioral change.
    #[serde(default)]
    pub mcp_global: crate::mcp::McpGlobalSettings,

    /// Local LLM (Ollama) auto-maintenance + user-stop intent persistence.
    #[serde(default)]
    pub local_llm: LocalLlmConfig,

    /// Speech-to-Text subsystem (cloud + local STT providers, IM auto-
    /// transcribe selection). See `crate::stt`.
    #[serde(default)]
    pub stt: crate::stt::SttConfig,
    /// Hooks subsystem — event → pluggable handler dispatch (Claude Code
    /// compatible). User scope only this phase. See `crate::hooks`.
    #[serde(default)]
    pub hooks: crate::hooks::HooksConfig,
    /// Master kill switch for all hooks (`disableAllHooks` in the protocol).
    #[serde(default)]
    pub disable_all_hooks: bool,
    /// Whether project/local scope hooks (`<cwd>/.hope-agent/hooks.json` and
    /// `hooks.local.json`) are loaded at all. Off by default: a repository's
    /// checked-in hooks must not auto-execute shell / HTTP / LLM / sub-agents
    /// just because a session's working dir points at it (supply-chain guard).
    /// User opts in globally via Settings → Hooks; user/managed scopes are
    /// unaffected.
    #[serde(default)]
    pub hooks_allow_project_scope: bool,

    /// Auto-update behavior: background check cadence, silent download, and
    /// user notification. Shared single source of truth for both the desktop
    /// (`@tauri-apps/plugin-updater`) and headless (`updater::auto_check`)
    /// paths. See `crate::updater::AutoUpdateConfig`.
    #[serde(default)]
    pub auto_update: crate::updater::AutoUpdateConfig,

    /// Per-function model overrides (issue #434). Currently just the vision
    /// bridge: when the main model can't see images, a separately-configured
    /// vision model transcribes them to text. Opt-in — `vision = None` keeps
    /// the existing drop-image + placeholder behavior. See `agent::vision_bridge`.
    #[serde(default)]
    pub function_models: FunctionModelsConfig,
}

// ── Per-function model overrides (issue #434) ───────────────────────

/// Model overrides keyed by function type. Extensible container so future
/// function→model routing (tool-use, reasoning, …) can be added without
/// reshaping `AppConfig`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionModelsConfig {
    /// Vision bridge model: transcribes images to text when the main model is
    /// text-only. `None` = bridge disabled (drop image + placeholder, as before).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision: Option<ActiveModel>,
    /// Default model chain for background/automation tasks (Recap, Dreaming,
    /// Knowledge Compile, Skills auto_review, Hooks `prompt` handler, Smart
    /// mode judge, session title, memory extraction, compaction summarizer —
    /// see `crate::automation`). `None` = fall through to the chat `active_model`
    /// / `fallback_models` chain. Independent of the main chat model so a
    /// cheaper/faster model can be dedicated to these one-shot calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation: Option<ModelChain>,
}

// ── Local LLM (Ollama) auto-maintenance ─────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalLlmConfig {
    /// Background watchdog that keeps default chat / embedding models
    /// preloaded and surfaces missing-file alerts.
    #[serde(default)]
    pub auto_maintenance: AutoMaintenanceConfig,
    /// Ollama model tags the user explicitly stopped via the UI. The
    /// auto-maintainer skips re-preloading any tag in this list — the user's
    /// stop intent always wins until they manually start the model again.
    #[serde(default)]
    pub user_stopped_models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoMaintenanceConfig {
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
}

impl Default for AutoMaintenanceConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            providers: Vec::new(),
            active_model: None,
            fallback_models: Vec::new(),
            default_agent_id: default_default_agent_id(),
            agent_order: Vec::new(),
            extra_skills_dirs: Vec::new(),
            disabled_skills: Vec::new(),
            skill_env_check: true,
            conditional_skills_enabled: true,
            embedding_models: Vec::new(),
            memory_embedding: crate::memory::EmbeddingSelection::default(),
            knowledge_embedding: crate::memory::EmbeddingSelection::default(),
            knowledge_chunk: crate::knowledge::ChunkConfig::default(),
            knowledge_search: crate::knowledge::KnowledgeSearchConfig::default(),
            knowledge_compile: crate::knowledge::KnowledgeCompileConfig::default(),
            knowledge_maintenance: crate::knowledge::maintenance::MaintenanceConfig::default(),
            knowledge_vision: crate::knowledge::KnowledgeVisionConfig::default(),
            note_tools: crate::knowledge::NoteToolsConfig::default(),
            knowledge_passive_recall: crate::knowledge::PassiveRecallConfig::default(),
            knowledge_media_retention: crate::knowledge::KnowledgeMediaRetentionConfig::default(),
            embedding: crate::memory::EmbeddingConfig::default(),
            memory_extract: crate::memory::MemoryExtractConfig::default(),
            memory: crate::memory::MemoryRuntimeConfig::default(),
            memory_selection: crate::memory::MemorySelectionConfig::default(),
            memory_budget: crate::memory::MemoryBudgetConfig::default(),
            dedup: crate::memory::DedupConfig::default(),
            memory_providers: crate::memory::ExternalMemoryProvidersConfig::default(),
            hybrid_search: crate::memory::HybridSearchConfig::default(),
            temporal_decay: crate::memory::TemporalDecayConfig::default(),
            mmr: crate::memory::MmrConfig::default(),
            multimodal: crate::memory::MultimodalConfig::default(),
            embedding_cache: crate::memory::EmbeddingCacheConfig::default(),
            web_search: crate::tools::web_search::WebSearchConfig::default(),
            web_fetch: crate::tools::web_fetch::WebFetchConfig::default(),
            ssrf: crate::security::ssrf::SsrfConfig::default(),
            filesystem: FilesystemConfig::default(),
            skill_env: std::collections::HashMap::new(),
            compact: crate::context_compact::CompactConfig::default(),
            session_title: crate::session_title::SessionTitleConfig::default(),
            notification: NotificationConfig::default(),
            startup_notification: StartupNotificationConfig::default(),
            image_generate: crate::tools::image_generate::ImageGenConfig::default(),
            audio_generate: crate::tools::audio_generate::AudioGenConfig::default(),
            issue_reporting: crate::issue_reporting::IssueReportingConfig::default(),
            canvas: crate::tools::canvas::CanvasConfig::default(),
            design: crate::design::DesignConfig::default(),
            browser: None,
            image: crate::tools::image::ImageToolConfig::default(),
            pdf: crate::tools::pdf::PdfToolConfig::default(),
            tool_timeout: default_tool_timeout(),
            timeout_policy: TimeoutPolicyConfig::default(),
            tool_result_disk_threshold: None,
            theme: default_theme(),
            enhanced_focus_indicators: false,
            language: default_language(),
            ui_effects_enabled: false,
            prevent_sleep: false,
            sidebar_ui_mode: default_sidebar_ui_mode(),
            proxy: ProxyConfig::default(),
            skill_prompt_budget: crate::skills::SkillPromptBudget::default(),
            skill_allow_bundled: Vec::new(),
            acp_control: crate::acp_control::AcpControlConfig::default(),
            shortcuts: ShortcutConfig::default(),
            quick_prompts: QuickPromptConfig::default(),
            plans_directory: None,
            temperature: None,
            reasoning_effort: default_reasoning_effort(),
            plan_subagent: false,
            ask_user_question_timeout_enabled: false,
            ask_user_question_timeout_secs: default_ask_user_question_timeout(),
            channels: crate::channel::ChannelStoreConfig::default(),
            deferred_tools: DeferredToolsConfig::default(),
            server: EmbeddedServerConfig::default(),
            recap: RecapConfig::default(),
            async_tools: AsyncToolsConfig::default(),
            cron: CronConfig::default(),
            awareness: crate::awareness::AwarenessConfig::default(),
            sprite: crate::sprite::SpriteConfig::default(),
            dreaming: crate::memory::dreaming::DreamingConfig::default(),
            skills: crate::skills::SkillsConfig::default(),
            recall_summary: crate::memory::RecallSummaryConfig::default(),
            tool_call_narration_enabled: default_tool_call_narration_enabled(),
            permission: crate::permission::PermissionGlobalConfig::default(),
            onboarding: OnboardingState::default(),
            mcp_servers: Vec::new(),
            mcp_global: crate::mcp::McpGlobalSettings::default(),
            local_llm: LocalLlmConfig::default(),
            stt: crate::stt::SttConfig::default(),
            hooks: crate::hooks::HooksConfig::default(),
            disable_all_hooks: false,
            hooks_allow_project_scope: false,
            auto_update: crate::updater::AutoUpdateConfig::default(),
            function_models: FunctionModelsConfig::default(),
        }
    }
}

#[cfg(test)]
mod deferred_tools_config_tests {
    use super::*;

    #[test]
    fn default_deferred_tools_preloads_low_frequency_tools() {
        let d = DeferredToolsConfig::default();
        assert!(d.enabled);
        assert_eq!(d.effective_mode(), DeferredToolsMode::Recommended);
        for name in DEFAULT_DEFERRED_TOOL_NAMES {
            assert!(
                d.tool_names.iter().any(|configured| configured == name),
                "missing default deferred tool {name}"
            );
        }
    }

    #[test]
    fn missing_deferred_tools_deserializes_to_default_policy() {
        let cfg: AppConfig = serde_json::from_str(r#"{"providers":[]}"#).unwrap();
        assert!(cfg.deferred_tools.enabled);
        assert_eq!(
            cfg.deferred_tools.effective_mode(),
            DeferredToolsMode::Recommended
        );
        assert_eq!(cfg.deferred_tools.tool_names, default_deferred_tool_names());
    }

    #[test]
    fn explicit_deferred_tools_opt_out_survives_deserialization() {
        let cfg: AppConfig = serde_json::from_str(
            r#"{"providers":[],"deferredTools":{"enabled":false,"toolNames":[]}}"#,
        )
        .unwrap();
        assert!(!cfg.deferred_tools.enabled);
        assert_eq!(
            cfg.deferred_tools.effective_mode(),
            DeferredToolsMode::Disabled
        );
        assert!(cfg.deferred_tools.tool_names.is_empty());
    }

    #[test]
    fn legacy_explicit_empty_list_remains_custom() {
        let cfg: AppConfig = serde_json::from_str(
            r#"{"providers":[],"deferredTools":{"enabled":true,"toolNames":[]}}"#,
        )
        .unwrap();
        assert_eq!(
            cfg.deferred_tools.effective_mode(),
            DeferredToolsMode::Custom
        );
    }
}

#[cfg(test)]
mod mcp_compat_tests {
    use super::*;

    // Backward-compat: a config.json produced before MCP landed must still
    // deserialize cleanly. The empty-object test is the minimum guarantee;
    // the providers-only test simulates what actual users have on disk.

    #[test]
    fn bare_providers_deserializes_with_mcp_defaults() {
        // `providers` is the only non-default field on AppConfig today; a
        // JSON with just that key is the minimum shape we want to guarantee
        // still parses after adding MCP fields.
        let cfg: AppConfig = serde_json::from_str(r#"{"providers":[]}"#)
            .expect("bare providers config should deserialize");
        assert!(cfg.mcp_servers.is_empty());
        assert!(cfg.mcp_global.enabled);
        assert_eq!(cfg.mcp_global.max_concurrent_calls, 8);
    }

    #[test]
    fn pre_mcp_config_deserializes() {
        // Representative subset of a real pre-MCP config.json: non-trivial
        // providers + theme + shortcuts but no mcp_* keys at all.
        let json = serde_json::json!({
            "providers": [],
            "theme": "dark",
            "language": "zh",
            "shortcuts": { "bindings": [] },
        });
        let cfg: AppConfig = serde_json::from_value(json).expect("pre-mcp config should load");
        assert_eq!(cfg.theme, "dark");
        assert_eq!(cfg.language, "zh");
        assert!(cfg.mcp_servers.is_empty());
        assert!(cfg.mcp_global.enabled);
    }

    #[test]
    fn mcp_servers_roundtrip() {
        use crate::mcp::{McpServerConfig, McpTransportSpec, McpTrustLevel};
        let server = McpServerConfig {
            id: "11111111-2222-3333-4444-555555555555".into(),
            name: "memory".into(),
            enabled: true,
            transport: McpTransportSpec::Stdio {
                command: "npx".into(),
                args: vec!["-y".into(), "@modelcontextprotocol/server-memory".into()],
                cwd: None,
            },
            env: Default::default(),
            headers: Default::default(),
            oauth: None,
            allowed_tools: vec![],
            denied_tools: vec![],
            connect_timeout_secs: 30,
            call_timeout_secs: 120,
            health_check_interval_secs: 60,
            max_concurrent_calls: 4,
            auto_approve: false,
            trust_level: McpTrustLevel::Untrusted,
            eager: false,
            deferred_tools: false,
            project_paths: vec![],
            description: Some("local knowledge base".into()),
            icon: None,
            created_at: 0,
            updated_at: 0,
            trust_acknowledged_at: None,
        };
        let mut cfg = AppConfig::default();
        cfg.mcp_servers.push(server.clone());

        let text = serde_json::to_string(&cfg).expect("serialize");
        let round: AppConfig = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(round.mcp_servers.len(), 1);
        assert_eq!(round.mcp_servers[0].name, "memory");
        assert!(matches!(
            round.mcp_servers[0].transport,
            McpTransportSpec::Stdio { .. }
        ));
    }

    #[test]
    fn mcp_global_disabled_roundtrip() {
        // Kill-switch scenario: user opts out via config file.
        let json = serde_json::json!({
            "providers": [],
            "mcpGlobal": { "enabled": false, "maxConcurrentCalls": 0 }
        });
        let cfg: AppConfig = serde_json::from_value(json).unwrap();
        assert!(!cfg.mcp_global.enabled);
        assert_eq!(cfg.mcp_global.max_concurrent_calls, 0);
        // Other defaults remain intact:
        assert_eq!(cfg.mcp_global.backoff_max_secs, 300);
    }
}

#[cfg(test)]
mod quick_prompt_config_tests {
    use super::*;

    #[test]
    fn missing_quick_prompts_deserializes_to_empty_config() {
        let cfg: AppConfig = serde_json::from_str(r#"{"providers":[]}"#).unwrap();
        assert!(cfg.quick_prompts.items.is_empty());
    }

    #[test]
    fn add_prompt_trims_titles_and_deduplicates_content() {
        let mut cfg = QuickPromptConfig::default();
        let first = cfg
            .add_prompt_with_created_at(
                "  First line\nSecond line  ",
                "2026-06-28T00:00:00Z".to_string(),
            )
            .unwrap();
        assert!(!first.duplicate);
        assert_eq!(first.item.title, "First line");
        assert_eq!(first.item.content, "First line\nSecond line");

        let duplicate = cfg
            .add_prompt_with_created_at(
                "First line\nSecond line",
                "2026-06-28T00:00:01Z".to_string(),
            )
            .unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(cfg.items.len(), 1);
        assert_eq!(duplicate.item.id, first.item.id);
    }

    #[test]
    fn add_prompt_rejects_empty_or_too_long_content() {
        let mut cfg = QuickPromptConfig::default();
        assert_eq!(
            cfg.add_prompt_with_created_at("   ", "2026-06-28T00:00:00Z".to_string())
                .unwrap_err(),
            QuickPromptError::Empty
        );

        let too_long = "x".repeat(MAX_QUICK_PROMPT_CONTENT_CHARS + 1);
        assert_eq!(
            cfg.add_prompt_with_created_at(&too_long, "2026-06-28T00:00:00Z".to_string())
                .unwrap_err(),
            QuickPromptError::TooLong {
                max_chars: MAX_QUICK_PROMPT_CONTENT_CHARS
            }
        );
    }
}

#[cfg(test)]
mod async_tools_defaults_tests {
    use super::*;

    #[test]
    fn max_concurrent_jobs_default_is_hardware_clamped() {
        // Hardware-derived default must always land in the [4, 16] band
        // regardless of core count: clamp(logical_cores - 2, 4, 16).
        let d = default_async_max_concurrent_jobs();
        assert!(
            (4..=16).contains(&d),
            "default {} out of clamp band [4,16]",
            d
        );
    }

    #[test]
    fn async_tools_uses_hardware_default_when_field_absent() {
        // A config without asyncTools.maxConcurrentJobs must fall back to the
        // hardware-derived default, not a hardcoded literal.
        let cfg: AppConfig = serde_json::from_str(r#"{"providers":[]}"#).unwrap();
        assert_eq!(
            cfg.async_tools.max_concurrent_jobs,
            default_async_max_concurrent_jobs()
        );
    }

    #[test]
    fn explicit_zero_unlimited_survives_deserialization() {
        // A user-set 0 (= unlimited) must NOT be overwritten by the default.
        let json = serde_json::json!({
            "providers": [],
            "asyncTools": { "maxConcurrentJobs": 0 }
        });
        let cfg: AppConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.async_tools.max_concurrent_jobs, 0);
    }

    #[test]
    fn default_impl_matches_serde_default() {
        // The hand-written `AsyncToolsConfig::default` (used by `AppConfig::default`)
        // and the `#[serde(default = ...)]` helper are two independent sources of the
        // same default; pin them together so a future edit to one literal can't
        // silently diverge from the other.
        assert_eq!(
            AsyncToolsConfig::default().max_concurrent_jobs,
            default_async_max_concurrent_jobs()
        );
        assert_eq!(
            AsyncToolsConfig::default().auto_background_secs,
            default_async_auto_background_secs()
        );
        assert_eq!(default_async_auto_background_secs(), 0);
        assert_eq!(
            AsyncToolsConfig::default().completion_merge_window_secs,
            default_async_completion_merge_window_secs()
        );
        assert_eq!(default_async_completion_merge_window_secs(), 3);
        assert_eq!(
            AsyncToolsConfig::default().max_concurrent_jobs_per_session,
            default_async_max_concurrent_jobs_per_session()
        );
        assert!(
            !AsyncToolsConfig::default().retry_enabled,
            "retry is opt-in (default off): eligible tools re-run and may re-bill"
        );
        assert_eq!(
            AsyncToolsConfig::default().max_retry_attempts,
            default_async_max_retry_attempts()
        );
        assert_eq!(default_async_max_retry_attempts(), 3);
        // R9 knobs: Default impl and serde-default helpers stay in lockstep.
        assert_eq!(
            AsyncToolsConfig::default().output_tail_bytes,
            default_async_output_tail_bytes()
        );
        assert_eq!(default_async_output_tail_bytes(), 8 * 1024);
        assert_eq!(
            AsyncToolsConfig::default().max_queued_jobs,
            default_async_max_queued_jobs()
        );
        assert_eq!(default_async_max_queued_jobs(), 256);
        assert_eq!(
            AsyncToolsConfig::default().wakeup_max_delay_secs,
            default_wakeup_max_delay_secs()
        );
        assert_eq!(default_wakeup_max_delay_secs(), 86_400);
        assert_eq!(
            AsyncToolsConfig::default().wakeup_max_pending_per_session,
            default_wakeup_max_pending_per_session()
        );
        assert_eq!(default_wakeup_max_pending_per_session(), 5);
    }

    #[test]
    fn cron_config_defaults_and_clamps() {
        // Defaults: cap 5 (0 = unlimited escape hatch), no cron-level timeout, grace 300s.
        let d = CronConfig::default();
        assert_eq!(d.max_concurrent, 5);
        assert_eq!(d.job_timeout_secs, 0);
        assert_eq!(d.at_grace_secs, 300);
        assert_eq!(d.effective_max_concurrent(), Some(5));
        assert_eq!(d.effective_at_grace_secs(), 300);
        assert_eq!(
            CronConfig {
                max_concurrent: 0,
                ..d.clone()
            }
            .effective_max_concurrent(),
            None
        );
        // Timeout clamps positive values to [30, 7200] at read; 0 means
        // unlimited / no cron-level timeout.
        assert_eq!(
            CronConfig {
                job_timeout_secs: 0,
                ..d.clone()
            }
            .effective_job_timeout_secs(),
            0
        );
        assert_eq!(
            CronConfig {
                job_timeout_secs: 5,
                ..d.clone()
            }
            .effective_job_timeout_secs(),
            30
        );
        assert_eq!(
            CronConfig {
                job_timeout_secs: 600,
                ..d.clone()
            }
            .effective_job_timeout_secs(),
            600
        );
        assert_eq!(
            CronConfig {
                job_timeout_secs: 999_999,
                ..d.clone()
            }
            .effective_job_timeout_secs(),
            7200
        );
        // Grace caps at 7 days but, unlike timeout, preserves 0 (= strict, no
        // late-fire) rather than flooring it.
        assert_eq!(
            CronConfig {
                at_grace_secs: 0,
                ..d.clone()
            }
            .effective_at_grace_secs(),
            0
        );
        assert_eq!(
            CronConfig {
                at_grace_secs: 999_999_999,
                ..d
            }
            .effective_at_grace_secs(),
            604_800
        );
    }

    #[test]
    fn r9_knobs_use_defaults_when_absent() {
        // Old configs (and partial writes) omit the R9 fields; serde fills them.
        let cfg: AppConfig = serde_json::from_str(r#"{"providers":[]}"#).unwrap();
        assert_eq!(cfg.async_tools.output_tail_bytes, 8 * 1024);
        assert_eq!(cfg.async_tools.max_queued_jobs, 256);
        assert_eq!(cfg.async_tools.wakeup_max_delay_secs, 86_400);
        assert_eq!(cfg.async_tools.wakeup_max_pending_per_session, 5);
    }

    #[test]
    fn retry_fields_use_defaults_when_absent() {
        let cfg: AppConfig = serde_json::from_str(r#"{"providers":[]}"#).unwrap();
        assert!(!cfg.async_tools.retry_enabled, "default off");
        assert_eq!(cfg.async_tools.max_retry_attempts, 3);
    }

    #[test]
    fn per_session_default_always_leaves_global_headroom() {
        // R7.1 review fix: the per-session default must stay STRICTLY below the
        // global default on every hardware tier, or the fairness tier no-ops
        // (a single session fills the whole global pool first). It is derived as
        // ~3/4 of the global cap (band [3,12]), floored at 2.
        let global = default_async_max_concurrent_jobs();
        let per_session = default_async_max_concurrent_jobs_per_session();
        assert!(
            per_session < global,
            "per-session default {per_session} must be below global default {global} or the cap never bites"
        );
        assert!(
            per_session >= 2,
            "per-session default {per_session} below floor 2"
        );
        assert!(
            (3..=12).contains(&per_session),
            "per-session default {per_session} out of expected band [3,12]"
        );
    }

    #[test]
    fn per_session_cap_zero_unlimited_survives_deserialization() {
        // A user-set 0 (= no per-session limit) must NOT be overwritten by the default.
        let json = serde_json::json!({
            "providers": [],
            "asyncTools": { "maxConcurrentJobsPerSession": 0 }
        });
        let cfg: AppConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.async_tools.max_concurrent_jobs_per_session, 0);
    }

    #[test]
    fn per_session_cap_uses_default_when_field_absent() {
        let cfg: AppConfig = serde_json::from_str(r#"{"providers":[]}"#).unwrap();
        assert_eq!(
            cfg.async_tools.max_concurrent_jobs_per_session,
            default_async_max_concurrent_jobs_per_session()
        );
    }
}
