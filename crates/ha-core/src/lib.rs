// Hope Agent Core — zero Tauri dependency
// All business logic lives here.

// ── Macros must come first ────────────────────────────────────────
#[macro_use]
pub mod logging;

// ── New abstractions ──────────────────────────────────────────────
pub mod event_bus;

// ── Initialization ────────────────────────────────────────────────
pub mod app_init;
pub mod async_jobs;
pub mod attachments;
pub mod globals;
mod util;

// ── Core modules (migrated from src-tauri) ────────────────────────
pub mod acp;
pub mod acp_control;
pub mod agent;
pub mod agent_config;
pub mod agent_loader;
pub mod ask_user;
pub mod awareness;
pub mod backup;
pub mod browser_state;
pub mod browser_ui;
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
pub mod filesystem;
pub mod guardian;
pub mod local_embedding;
pub mod local_llm;
pub mod local_model_jobs;

pub mod mcp;
pub mod memory;
pub mod memory_extract;
pub mod oauth;
pub mod onboarding;
pub mod openclaw_import;
pub mod paths;
pub mod permission;
pub mod permissions;
pub mod plan;
pub mod platform;
pub mod process_registry;
pub mod project;
pub mod provider;
pub mod recap;
pub mod runtime_lock;
pub mod runtime_tasks;
pub mod sandbox;
pub mod security;
pub mod self_diagnosis;
pub mod server_status;
pub mod service_install;
pub mod session;
pub mod session_title;
pub mod skills;
pub mod slash_commands;
pub mod subagent;
pub mod system_prompt;
pub mod team;
pub mod tools;
pub mod url_preview;
pub mod user_config;
pub mod weather;
#[cfg(target_os = "macos")]
pub mod weather_location_macos;

// ── Re-exports ────────────────────────────────────────────────────
pub use app_init::{
    build_app_state, init_app_state, init_runtime, start_background_tasks,
    start_minimal_background_tasks,
};
#[allow(deprecated)]
pub use globals::{
    get_acp_manager, get_app_handle, get_cached_agent, get_channel_cancels, get_channel_db,
    get_channel_registry, get_codex_token_cache, get_cron_db, get_event_bus, get_log_db,
    get_logger, get_memory_backend, get_project_db, get_reasoning_effort_cell, get_session_db,
    get_subagent_cancels, require_cached_agent, require_channel_cancels, require_codex_token_cache,
    require_cron_db, require_log_db, require_logger, require_project_db,
    require_reasoning_effort_cell, require_session_db, require_subagent_cancels, set_event_bus,
    AppState, ACP_MANAGER, APP_LOGGER, CACHED_AGENT, CHANNEL_CANCELS, CHANNEL_DB, CHANNEL_REGISTRY,
    CODEX_TOKEN_CACHE, CRON_DB, EVENT_BUS, LOG_DB, MEMORY_BACKEND, PROJECT_DB, REASONING_EFFORT,
    SESSION_DB, SUBAGENT_CANCELS,
};
pub use util::*;
