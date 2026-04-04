use crate::acp_control;
use crate::agent::AssistantAgent;
use crate::channel;
use crate::cron;
use crate::logging::{AppLogger, LogDB};
use crate::memory;
use crate::oauth::TokenData;
use crate::provider::ProviderStore;
use crate::session::SessionDB;
use crate::subagent;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::Mutex;

// ── Global statics (OnceLock) ──────────────────────────────────

pub(crate) static APP_HANDLE: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();
pub(crate) static APP_LOGGER: std::sync::OnceLock<AppLogger> = std::sync::OnceLock::new();
pub(crate) static MEMORY_BACKEND: std::sync::OnceLock<Arc<dyn memory::MemoryBackend>> =
    std::sync::OnceLock::new();
pub(crate) static CRON_DB: std::sync::OnceLock<Arc<cron::CronDB>> = std::sync::OnceLock::new();
pub(crate) static SESSION_DB: std::sync::OnceLock<Arc<SessionDB>> = std::sync::OnceLock::new();
pub(crate) static SUBAGENT_CANCELS: std::sync::OnceLock<Arc<subagent::SubagentCancelRegistry>> =
    std::sync::OnceLock::new();
pub(crate) static ACP_MANAGER: std::sync::OnceLock<Arc<acp_control::AcpSessionManager>> =
    std::sync::OnceLock::new();
pub(crate) static CHANNEL_REGISTRY: std::sync::OnceLock<Arc<channel::ChannelRegistry>> =
    std::sync::OnceLock::new();
pub(crate) static CHANNEL_DB: std::sync::OnceLock<Arc<channel::ChannelDB>> =
    std::sync::OnceLock::new();

// ── Accessor functions ─────────────────────────────────────────

/// Get stored AppLogger for global logging
pub fn get_logger() -> Option<&'static AppLogger> {
    APP_LOGGER.get()
}

/// Get stored AppHandle for global event emission (e.g., command approval)
pub fn get_app_handle() -> Option<&'static tauri::AppHandle> {
    APP_HANDLE.get()
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

// ── Application state ──────────────────────────────────────────

pub(crate) struct AppState {
    pub(crate) agent: Mutex<Option<AssistantAgent>>,
    pub(crate) auth_result: Arc<Mutex<Option<anyhow::Result<TokenData>>>>,
    /// Provider configuration store
    pub(crate) provider_store: Mutex<ProviderStore>,
    /// Reasoning effort for Codex models
    pub(crate) reasoning_effort: Mutex<String>,
    /// Store token info so we can rebuild agent when model changes
    pub(crate) codex_token: Mutex<Option<(String, String)>>, // (access_token, account_id)
    /// Currently active agent ID
    pub(crate) current_agent_id: Mutex<String>,
    /// Session database
    pub(crate) session_db: Arc<SessionDB>,
    /// Cancel flag for stopping ongoing chat
    pub(crate) chat_cancel: Arc<AtomicBool>,
    /// Log database
    pub(crate) log_db: Arc<LogDB>,
    /// Async logger
    pub(crate) logger: AppLogger,
    /// Cron database
    pub(crate) cron_db: Arc<cron::CronDB>,
    /// Sub-agent cancel registry
    pub(crate) subagent_cancels: Arc<subagent::SubagentCancelRegistry>,
    /// Channel stream cancel registry
    pub(crate) channel_cancels: Arc<channel::ChannelCancelRegistry>,
}
