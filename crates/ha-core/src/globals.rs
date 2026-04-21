use crate::acp_control;
use crate::agent::AssistantAgent;
use crate::channel;
use crate::cron;
use crate::event_bus::EventBus;
use crate::logging::{AppLogger, LogDB};
use crate::memory;
use crate::oauth::TokenData;
use crate::project::ProjectDB;
use crate::session::SessionDB;
use crate::subagent;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::Mutex;

// ── Global statics (OnceLock) ──────────────────────────────────
//
// Every cross-runtime singleton — anything ha-core modules might need from
// desktop / HTTP / IM-channel / ACP / cron paths — lives here as a
// `OnceLock<Arc<…>>`. `AppState` below is a Tauri convenience aggregate
// that shares the same Arcs via `init_app_state()`.

pub static EVENT_BUS: std::sync::OnceLock<Arc<dyn EventBus>> = std::sync::OnceLock::new();
pub static APP_LOGGER: std::sync::OnceLock<AppLogger> = std::sync::OnceLock::new();
pub static MEMORY_BACKEND: std::sync::OnceLock<Arc<dyn memory::MemoryBackend>> =
    std::sync::OnceLock::new();
pub static CRON_DB: std::sync::OnceLock<Arc<cron::CronDB>> = std::sync::OnceLock::new();
pub static SESSION_DB: std::sync::OnceLock<Arc<SessionDB>> = std::sync::OnceLock::new();
pub static PROJECT_DB: std::sync::OnceLock<Arc<ProjectDB>> = std::sync::OnceLock::new();
pub static SUBAGENT_CANCELS: std::sync::OnceLock<Arc<subagent::SubagentCancelRegistry>> =
    std::sync::OnceLock::new();
pub static ACP_MANAGER: std::sync::OnceLock<Arc<acp_control::AcpSessionManager>> =
    std::sync::OnceLock::new();
pub static CHANNEL_REGISTRY: std::sync::OnceLock<Arc<channel::ChannelRegistry>> =
    std::sync::OnceLock::new();
pub static CHANNEL_DB: std::sync::OnceLock<Arc<channel::ChannelDB>> = std::sync::OnceLock::new();
pub static LOG_DB: std::sync::OnceLock<Arc<LogDB>> = std::sync::OnceLock::new();

pub static CHANNEL_CANCELS: std::sync::OnceLock<Arc<channel::ChannelCancelRegistry>> =
    std::sync::OnceLock::new();

/// Disk (`crate::oauth::load_token()`) is the source of truth; this cache
/// lets hot paths avoid a disk read and gives the login flow a publish
/// point for freshly-minted pairs.
pub static CODEX_TOKEN_CACHE: std::sync::OnceLock<Arc<Mutex<Option<(String, String)>>>> =
    std::sync::OnceLock::new();

pub static REASONING_EFFORT: std::sync::OnceLock<Arc<Mutex<String>>> = std::sync::OnceLock::new();

/// Best-effort convenience: the primary chat path rebuilds agents per
/// request from config + DB history, so a stale or empty cache is a
/// missed optimization, not a correctness bug.
pub static CACHED_AGENT: std::sync::OnceLock<Arc<Mutex<Option<AssistantAgent>>>> =
    std::sync::OnceLock::new();

/// Registry for idle extraction delayed tasks, keyed by session_id.
/// Each entry holds (AbortHandle, agent_id, updated_at_snapshot) for deferred extraction.
pub static IDLE_EXTRACT_HANDLES: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, (tokio::task::AbortHandle, String, String)>>,
> = std::sync::OnceLock::new();

// ── Accessor functions ─────────────────────────────────────────

/// Get stored AppLogger for global logging
pub fn get_logger() -> Option<&'static AppLogger> {
    APP_LOGGER.get()
}

/// Get stored EventBus for global event emission (e.g., command approval)
pub fn get_event_bus() -> Option<&'static Arc<dyn EventBus>> {
    EVENT_BUS.get()
}

/// Set the global EventBus instance (called once during app initialization)
pub fn set_event_bus(bus: Arc<dyn EventBus>) {
    let _ = EVENT_BUS.set(bus);
}

/// Deprecated: returns `None` unconditionally.
/// Callers should migrate to `get_event_bus()` + `EventBus::emit()`.
#[deprecated(
    note = "Use get_event_bus() instead — Tauri AppHandle is no longer available in ha-core"
)]
pub fn get_app_handle() -> Option<&'static Arc<dyn EventBus>> {
    None
}

/// Get stored MemoryBackend for memory operations
pub fn get_memory_backend() -> Option<&'static Arc<dyn memory::MemoryBackend>> {
    MEMORY_BACKEND.get()
}

/// Get stored CronDB for cron operations (used by agent tool)
pub fn get_cron_db() -> Option<&'static Arc<cron::CronDB>> {
    CRON_DB.get()
}

/// Get stored SessionDB for sub-agent operations
pub fn get_session_db() -> Option<&'static Arc<SessionDB>> {
    SESSION_DB.get()
}

/// Get stored ProjectDB for project CRUD + file management
pub fn get_project_db() -> Option<&'static Arc<ProjectDB>> {
    PROJECT_DB.get()
}

/// Get stored SubagentCancelRegistry for sub-agent cancellation
pub fn get_subagent_cancels() -> Option<&'static Arc<subagent::SubagentCancelRegistry>> {
    SUBAGENT_CANCELS.get()
}

/// Get stored AcpSessionManager for ACP control plane operations
pub fn get_acp_manager() -> Option<&'static Arc<acp_control::AcpSessionManager>> {
    ACP_MANAGER.get()
}

/// Get stored ChannelRegistry for IM channel operations
pub fn get_channel_registry() -> Option<&'static Arc<channel::ChannelRegistry>> {
    CHANNEL_REGISTRY.get()
}

/// Get stored ChannelDB for channel conversation management
pub fn get_channel_db() -> Option<&'static Arc<channel::ChannelDB>> {
    CHANNEL_DB.get()
}

/// Get stored LogDB for log persistence (separate from the [`AppLogger`]
/// async writer — routes that page logs need the DB handle directly).
pub fn get_log_db() -> Option<&'static Arc<LogDB>> {
    LOG_DB.get()
}

/// Get stored ChannelCancelRegistry for IM-channel stream cancellation
pub fn get_channel_cancels() -> Option<&'static Arc<channel::ChannelCancelRegistry>> {
    CHANNEL_CANCELS.get()
}

/// Get stored in-memory Codex OAuth token cache.
/// Disk is the source of truth — use this only as a fast-path snapshot.
pub fn get_codex_token_cache() -> Option<&'static Arc<Mutex<Option<(String, String)>>>> {
    CODEX_TOKEN_CACHE.get()
}

/// Get stored runtime reasoning-effort preference cell.
pub fn get_reasoning_effort_cell() -> Option<&'static Arc<Mutex<String>>> {
    REASONING_EFFORT.get()
}

/// Get stored cached AssistantAgent (best-effort; may be stale or empty).
pub fn get_cached_agent() -> Option<&'static Arc<Mutex<Option<AssistantAgent>>>> {
    CACHED_AGENT.get()
}

// ── Canonical `require_*` accessors ────────────────────────────
//
// Each returns `anyhow::Result<&'static Arc<T>>` with a stable "<X> not
// initialized" message so call sites share one error shape. `.cloned()`
// at the callsite when you need ownership; `.map_err(...)` at the HTTP
// boundary to turn anyhow into AppError / String.

macro_rules! require_accessor {
    ($name:ident, $getter:ident, $ret:ty, $label:literal) => {
        pub fn $name() -> anyhow::Result<&'static $ret> {
            $getter().ok_or_else(|| anyhow::anyhow!(concat!($label, " not initialized")))
        }
    };
}

require_accessor!(require_logger, get_logger, AppLogger, "AppLogger");
require_accessor!(
    require_session_db,
    get_session_db,
    Arc<SessionDB>,
    "Session DB"
);
require_accessor!(
    require_project_db,
    get_project_db,
    Arc<ProjectDB>,
    "Project DB"
);
require_accessor!(require_cron_db, get_cron_db, Arc<cron::CronDB>, "Cron DB");
require_accessor!(require_log_db, get_log_db, Arc<LogDB>, "Log DB");
require_accessor!(
    require_subagent_cancels,
    get_subagent_cancels,
    Arc<subagent::SubagentCancelRegistry>,
    "Sub-agent cancel registry"
);
require_accessor!(
    require_channel_cancels,
    get_channel_cancels,
    Arc<channel::ChannelCancelRegistry>,
    "Channel cancel registry"
);
require_accessor!(
    require_codex_token_cache,
    get_codex_token_cache,
    Arc<Mutex<Option<(String, String)>>>,
    "Codex token cache"
);
require_accessor!(
    require_reasoning_effort_cell,
    get_reasoning_effort_cell,
    Arc<Mutex<String>>,
    "Reasoning effort cell"
);
require_accessor!(
    require_cached_agent,
    get_cached_agent,
    Arc<Mutex<Option<AssistantAgent>>>,
    "Cached agent cell"
);

// ── Application state ──────────────────────────────────────────
//
// Tauri convenience aggregate served to commands via `State<'_, AppState>`.
// Every `Arc<…>` field that has a matching OnceLock above shares the same
// allocation — `init_app_state()` enforces this with a `debug_assert!`
// so a drift between the two access styles becomes an immediate panic.

pub struct AppState {
    pub agent: Arc<Mutex<Option<AssistantAgent>>>,
    /// Desktop OAuth login rendezvous (no cross-runtime consumer).
    pub auth_result: Arc<Mutex<Option<anyhow::Result<TokenData>>>>,
    pub reasoning_effort: Arc<Mutex<String>>,
    pub codex_token: Arc<Mutex<Option<(String, String)>>>,
    /// Desktop-only.
    pub current_agent_id: Mutex<String>,
    pub session_db: Arc<SessionDB>,
    pub project_db: Arc<ProjectDB>,
    /// Desktop chat turn cancel. IM-channel cancels live in [`CHANNEL_CANCELS`].
    pub chat_cancel: Arc<AtomicBool>,
    pub log_db: Arc<LogDB>,
    pub logger: AppLogger,
    pub cron_db: Arc<cron::CronDB>,
    pub subagent_cancels: Arc<subagent::SubagentCancelRegistry>,
    pub channel_cancels: Arc<channel::ChannelCancelRegistry>,
}
