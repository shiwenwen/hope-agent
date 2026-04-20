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
pub static APP_STATE: std::sync::OnceLock<Arc<AppState>> = std::sync::OnceLock::new();

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

/// Get stored AppState for global application state access
pub fn get_app_state() -> Option<&'static Arc<AppState>> {
    APP_STATE.get()
}

/// Set the global AppState instance (called once during app initialization)
pub fn set_app_state(state: Arc<AppState>) {
    let _ = APP_STATE.set(state);
}

// ── Application state ──────────────────────────────────────────

pub struct AppState {
    pub agent: Mutex<Option<AssistantAgent>>,
    pub auth_result: Arc<Mutex<Option<anyhow::Result<TokenData>>>>,
    /// Reasoning effort for Codex models
    pub reasoning_effort: Mutex<String>,
    /// Store token info so we can rebuild agent when model changes
    pub codex_token: Mutex<Option<(String, String)>>, // (access_token, account_id)
    /// Currently active agent ID
    pub current_agent_id: Mutex<String>,
    /// Session database
    pub session_db: Arc<SessionDB>,
    /// Project database (shares the same SQLite file as `session_db`)
    pub project_db: Arc<ProjectDB>,
    /// Cancel flag for stopping ongoing chat
    pub chat_cancel: Arc<AtomicBool>,
    /// Log database
    pub log_db: Arc<LogDB>,
    /// Async logger
    pub logger: AppLogger,
    /// Cron database
    pub cron_db: Arc<cron::CronDB>,
    /// Sub-agent cancel registry
    pub subagent_cancels: Arc<subagent::SubagentCancelRegistry>,
    /// Channel stream cancel registry
    pub channel_cancels: Arc<channel::ChannelCancelRegistry>,
}
