// OpenComputer Core — zero Tauri dependency
// All business logic lives here.

// ── Macros must come first ────────────────────────────────────────
#[macro_use]
pub mod logging;

// ── New abstractions ──────────────────────────────────────────────
pub mod event_bus;

// ── Initialization ────────────────────────────────────────────────
pub mod app_init;
pub mod globals;
mod util;

// ── Core modules (migrated from src-tauri) ────────────────────────
pub mod acp;
pub mod acp_control;
pub mod agent;
pub mod agent_config;
pub mod agent_loader;
pub mod backup;
pub mod browser_state;
pub mod canvas_db;
pub mod channel;
pub mod chat_engine;
pub mod config;
pub mod context_compact;
pub mod crash_journal;
pub mod cron;
pub mod dashboard;
pub mod dev_tools;
pub mod docker;
pub mod failover;
pub mod file_extract;
pub mod guardian;

pub mod memory;
pub mod memory_extract;
pub mod oauth;
pub mod paths;
pub mod permissions;
pub mod plan;
pub mod process_registry;
pub mod provider;
pub mod sandbox;
pub mod self_diagnosis;
pub mod service_install;
pub mod session;
pub mod skills;
pub mod slash_commands;
pub mod subagent;
pub mod system_prompt;
pub mod tools;
pub mod url_preview;
pub mod user_config;
pub mod weather;
#[cfg(target_os = "macos")]
pub mod weather_location_macos;

// ── Re-exports ────────────────────────────────────────────────────
pub use util::*;
#[allow(deprecated)]
pub use globals::{
    get_acp_manager, get_app_handle, get_app_state, get_event_bus, get_channel_db,
    get_channel_registry, get_cron_db, get_logger, get_memory_backend, get_session_db,
    get_subagent_cancels, set_app_state, set_event_bus, AppState,
    APP_LOGGER, ACP_MANAGER, CHANNEL_DB, CHANNEL_REGISTRY, CRON_DB, EVENT_BUS,
    MEMORY_BACKEND, SESSION_DB, SUBAGENT_CANCELS, APP_STATE,
};
pub use app_init::{init_app_state, start_background_tasks};
