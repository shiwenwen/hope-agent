#[macro_use]
mod logging;
pub mod acp;
pub(crate) mod acp_control;
mod agent;
mod agent_config;
mod agent_loader;
mod browser_state;
mod canvas_db;
mod commands;
mod cron;
mod dev_tools;
mod docker;
mod memory;
mod memory_extract;
mod failover;
mod file_extract;
mod oauth;
pub mod paths;
mod process_registry;
pub mod provider;
mod sandbox;
pub mod session;
mod skills;
mod subagent;
mod system_prompt;
mod permissions;
mod tools;
mod user_config;
mod context_compact;
mod dashboard;
mod slash_commands;
pub mod crash_journal;
pub mod backup;
pub mod self_diagnosis;
mod plan;

use agent::AssistantAgent;
use oauth::TokenData;
use provider::ProviderStore;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::Mutex;
use session::SessionDB;
use logging::{LogDB, AppLogger};

/// Truncate a string to at most `max_bytes` bytes on a valid UTF-8 char boundary.
pub fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    // floor_char_boundary is nightly-only, so do it manually
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

static APP_HANDLE: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();
static APP_LOGGER: std::sync::OnceLock<AppLogger> = std::sync::OnceLock::new();
static MEMORY_BACKEND: std::sync::OnceLock<Arc<dyn memory::MemoryBackend>> = std::sync::OnceLock::new();
static CRON_DB: std::sync::OnceLock<Arc<cron::CronDB>> = std::sync::OnceLock::new();
static SESSION_DB: std::sync::OnceLock<Arc<SessionDB>> = std::sync::OnceLock::new();
static SUBAGENT_CANCELS: std::sync::OnceLock<Arc<subagent::SubagentCancelRegistry>> = std::sync::OnceLock::new();
static ACP_MANAGER: std::sync::OnceLock<Arc<acp_control::AcpSessionManager>> = std::sync::OnceLock::new();

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

/// If SearXNG is docker-managed and enabled, auto-start the container on app launch.
fn auto_start_searxng_docker() {
    let store = match provider::load_store() {
        Ok(s) => s,
        Err(_) => return,
    };

    // Check: docker-managed + SearXNG enabled
    let docker_managed = store.web_search.searxng_docker_managed.unwrap_or(false);
    let searxng_enabled = store.web_search.providers.iter()
        .any(|e| e.id == tools::web_search::WebSearchProvider::Searxng && e.enabled);

    if !docker_managed || !searxng_enabled {
        return;
    }

    // Spawn background task — don't block app startup
    std::thread::spawn(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime for SearXNG auto-start");
        rt.block_on(async {
            let status = docker::status().await;
            if !status.docker_installed || status.docker_not_running {
                if let Some(logger) = get_logger() {
                    logger.log("warn", "docker", "auto_start", "Docker not available, skipping SearXNG auto-start", None, None, None);
                }
                return;
            }
            if status.container_running && status.health_ok {
                // Already running, nothing to do
                return;
            }
            if status.container_exists && !status.container_running {
                if let Some(logger) = get_logger() {
                    logger.log("info", "docker", "auto_start", "Auto-starting SearXNG container...", None, None, None);
                }
                if let Err(e) = docker::start().await {
                    if let Some(logger) = get_logger() {
                        logger.log("error", "docker", "auto_start", "Failed to auto-start SearXNG", Some(e.to_string()), None, None);
                    }
                }
            }
        });
    });
}

pub(crate) struct AppState {
    pub(crate) agent: Mutex<Option<AssistantAgent>>,
    pub(crate) auth_result: Arc<Mutex<Option<anyhow::Result<TokenData>>>>,
    /// Provider configuration store
    pub(crate) provider_store: Mutex<ProviderStore>,
    /// Reasoning effort for Codex models
    pub(crate) reasoning_effort: Mutex<String>,
    /// Store token info so we can rebuild agent when model changes
    pub(crate) codex_token: Mutex<Option<(String, String)>>,  // (access_token, account_id)
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
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize directory structure
    // NOTE: log::error! is intentional here — AppLogger is not yet initialized at this point
    if let Err(e) = paths::ensure_dirs() {
        log::error!("Failed to initialize data directories: {}", e);
    }

    // Ensure default agent exists
    if let Err(e) = agent_loader::ensure_default_agent() {
        log::error!("Failed to ensure default agent: {}", e);
    }

    // Load provider store at startup
    let initial_store = provider::load_store().unwrap_or_default();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // When a second instance is launched, focus the existing window
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_process::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app_handle, _shortcut, event| {
                    if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        use tauri::Manager;
                        if let Some(window) = app_handle.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                        use tauri::Emitter;
                        let _ = app_handle.emit("quick-chat-toggle", ());
                    }
                })
                .build(),
        )
        .setup(|app| {
            // Store global AppHandle for event emission
            let _ = APP_HANDLE.set(app.handle().clone());
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // Fix macOS theme-aware background to prevent flash on window resize
            #[cfg(target_os = "macos")]
            {
                use tauri::Manager;
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.with_webview(|webview| unsafe {
                        let ns_window: &objc2_app_kit::NSWindow =
                            &*webview.ns_window().cast();
                        // Detect system dark mode via appearance name
                        let is_dark = {
                            use objc2_app_kit::NSAppearanceCustomization;
                            let appearance = ns_window.effectiveAppearance();
                            let name = appearance.name();
                            name.to_string().contains("Dark")
                        };
                        let (r, g, b) = if is_dark {
                            (15.0 / 255.0, 15.0 / 255.0, 15.0 / 255.0)
                        } else {
                            (1.0, 1.0, 1.0)
                        };
                        let bg_color =
                            objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0);
                        ns_window.setBackgroundColor(Some(&bg_color));
                    });
                }
            }

            // Start cron scheduler on dedicated thread with its own tokio runtime
            if let (Some(cron_db), Ok(db_path)) = (CRON_DB.get(), session::db_path()) {
                if let Ok(session_db) = SessionDB::open(&db_path) {
                    let _handle = cron::start_scheduler(
                        cron_db.clone(),
                        Arc::new(session_db),
                    );
                    // Thread runs until app exits
                }
            }

            // Auto-start Docker SearXNG if previously configured
            auto_start_searxng_docker();

            // Register global shortcut (Alt+Space / Option+Space)
            {
                use tauri_plugin_global_shortcut::GlobalShortcutExt;
                let shortcut = "Alt+Space".parse::<tauri_plugin_global_shortcut::Shortcut>()
                    .map_err(|e| format!("Failed to parse shortcut: {}", e))?;
                app.global_shortcut().register(shortcut)
                    .map_err(|e| format!("Failed to register global shortcut: {}", e))?;
            }

            Ok(())
        })
        .manage({
            // Initialize the SessionDB
            let db_path = session::db_path().expect("Failed to resolve database path");
            let session_db = Arc::new(
                SessionDB::open(&db_path).expect("Failed to open session database")
            );

            // Initialize the LogDB and AppLogger
            let log_db_path = logging::db_path().expect("Failed to resolve log database path");
            let log_db = Arc::new(
                LogDB::open(&log_db_path).expect("Failed to open log database")
            );

            // Load log config and cleanup old logs
            let log_config = logging::load_log_config().unwrap_or_default();
            let _ = log_db.cleanup_old(log_config.max_age_days);
            // Clean up old log files
            let _ = logging::cleanup_old_log_files(log_config.max_age_days);
            let logs_dir = paths::logs_dir().expect("Failed to resolve logs directory");
            let logger = AppLogger::new(log_db.clone(), logs_dir);
            logger.update_config(log_config);

            // Store logger globally for access from non-State contexts
            let _ = APP_LOGGER.set(logger.clone());

            // Initialize the MemoryDB
            let memory_db_path = paths::memory_db_path().expect("Failed to resolve memory database path");
            let memory_backend: Arc<dyn memory::MemoryBackend> = Arc::new(
                memory::SqliteMemoryBackend::open(&memory_db_path).expect("Failed to open memory database")
            );
            let _ = MEMORY_BACKEND.set(memory_backend);

            // Auto-initialize embedder if enabled in config
            if let Some(backend) = MEMORY_BACKEND.get() {
                match provider::load_store() {
                    Ok(store) if store.embedding.enabled => {
                        match memory::create_embedding_provider(&store.embedding) {
                            Ok(emb_provider) => {
                                backend.set_embedder(emb_provider);
                                logger.log("info", "memory", "embedding", "Embedding provider auto-initialized on startup", None, None, None);
                            }
                            Err(e) => {
                                logger.log("warn", "memory", "embedding", &format!("Failed to auto-initialize embedding provider: {}", e), None, None, None);
                            }
                        }
                    }
                    _ => {} // Embedding not enabled or config load failed — skip silently
                }
            }

            // Initialize the CronDB (scheduler started in .setup() where tokio runtime is available)
            let cron_db_path = paths::cron_db_path().expect("Failed to resolve cron database path");
            let cron_db = Arc::new(
                cron::CronDB::open(&cron_db_path).expect("Failed to open cron database")
            );
            let _ = CRON_DB.set(cron_db.clone());

            // Log system startup
            logger.log("info", "system", "lib::run", "OpenComputer started", None, None, None);

            // Initialize sub-agent cancel registry
            let subagent_cancels = Arc::new(subagent::SubagentCancelRegistry::new());
            let _ = SUBAGENT_CANCELS.set(subagent_cancels.clone());
            let _ = SESSION_DB.set(session_db.clone());

            // Clean up orphan sub-agent runs from previous app session
            subagent::cleanup_orphan_runs(&session_db);

            // Initialize ACP control plane
            {
                let store = provider::load_store().unwrap_or_default();
                if store.acp_control.enabled {
                    let registry = Arc::new(acp_control::AcpRuntimeRegistry::new());
                    let registry_clone = Arc::clone(&registry);
                    let acp_config = store.acp_control.clone();
                    // Auto-discover backends in background
                    tokio::spawn(async move {
                        acp_control::registry::auto_discover_and_register(&registry_clone, &acp_config).await;
                    });
                    let manager = Arc::new(acp_control::AcpSessionManager::new(registry));
                    let _ = ACP_MANAGER.set(manager);
                }
            }

            AppState {
                agent: Mutex::new(None),
                auth_result: Arc::new(Mutex::new(None)),
                provider_store: Mutex::new(initial_store),
                reasoning_effort: Mutex::new("medium".to_string()),
                codex_token: Mutex::new(None),
                current_agent_id: Mutex::new("default".to_string()),
                session_db,
                chat_cancel: Arc::new(AtomicBool::new(false)),
                log_db,
                logger,
                cron_db,
                subagent_cancels,
            }
        })
        .invoke_handler(tauri::generate_handler![
            // Provider management
            commands::provider::get_providers,
            commands::provider::add_provider,
            commands::provider::update_provider,
            commands::provider::reorder_providers,
            commands::provider::delete_provider,
            commands::provider::test_provider,
            commands::provider::test_model,
            commands::provider::test_embedding,
            commands::provider::test_image_generate,
            commands::provider::get_available_models,
            commands::provider::get_active_model,
            commands::provider::set_active_model,
            commands::provider::get_fallback_models,
            commands::provider::set_fallback_models,
            commands::provider::has_providers,
            // Legacy auth
            commands::auth::initialize_agent,
            commands::auth::start_codex_auth,
            commands::auth::check_auth_status,
            commands::auth::finalize_codex_auth,
            commands::auth::try_restore_session,
            commands::auth::logout_codex,
            // Model & settings (legacy)
            commands::auth::get_codex_models,
            commands::auth::get_current_settings,
            commands::auth::set_codex_model,
            commands::auth::set_reasoning_effort,
            // Chat
            commands::chat::save_attachment,
            commands::chat::chat,
            commands::chat::stop_chat,
            // Command approval
            commands::chat::respond_to_approval,
            // Tools info
            commands::chat::list_builtin_tools,
            // Skills
            commands::skills::get_skills,
            commands::skills::get_skill_detail,
            commands::skills::get_extra_skills_dirs,
            commands::skills::add_extra_skills_dir,
            commands::skills::remove_extra_skills_dir,
            commands::skills::toggle_skill,
            commands::skills::get_skill_env_check,
            commands::skills::set_skill_env_check,
            commands::skills::get_skill_env,
            commands::skills::set_skill_env_var,
            commands::skills::remove_skill_env_var,
            commands::skills::get_skills_env_status,
            commands::skills::get_skills_status,
            commands::skills::install_skill_dependency,
            commands::misc::open_directory,
            commands::misc::reveal_in_folder,
            commands::misc::open_url,
            commands::misc::write_export_file,
            // Agent management
            commands::agent_mgmt::list_agents,
            commands::agent_mgmt::get_agent_config,
            commands::agent_mgmt::get_agent_markdown,
            commands::agent_mgmt::save_agent_config_cmd,
            commands::agent_mgmt::save_agent_markdown,
            commands::agent_mgmt::delete_agent,
            commands::agent_mgmt::get_agent_template,
            // Memory management
            commands::memory::memory_add,
            commands::memory::memory_update,
            commands::memory::memory_toggle_pin,
            commands::memory::memory_delete,
            commands::memory::memory_get,
            commands::memory::memory_list,
            commands::memory::memory_search,
            commands::memory::memory_count,
            commands::memory::memory_export,
            commands::memory::memory_find_similar,
            commands::memory::memory_delete_batch,
            commands::memory::memory_import,
            commands::memory::memory_reembed,
            commands::memory::get_global_memory_md,
            commands::memory::save_global_memory_md,
            commands::memory::get_agent_memory_md,
            commands::memory::save_agent_memory_md,
            commands::config::get_web_search_config,
            commands::config::save_web_search_config,
            commands::config::get_web_fetch_config,
            commands::config::save_web_fetch_config,
            commands::config::get_image_generate_config,
            commands::config::save_image_generate_config,
            commands::config::get_proxy_config,
            commands::config::save_proxy_config,
            commands::config::test_proxy,
            commands::docker::searxng_docker_status,
            commands::docker::searxng_docker_deploy,
            commands::docker::searxng_docker_start,
            commands::docker::searxng_docker_stop,
            commands::docker::searxng_docker_remove,
            commands::memory::memory_stats,
            commands::memory::get_extract_config,
            commands::memory::save_extract_config,
            commands::memory::get_dedup_config,
            commands::memory::save_dedup_config,
            commands::memory::get_embedding_config,
            commands::memory::save_embedding_config,
            commands::memory::get_embedding_presets,
            commands::config::get_compact_config,
            commands::config::save_compact_config,
            commands::config::get_notification_config,
            commands::config::save_notification_config,
            commands::config::compact_context_now,
            commands::memory::list_local_embedding_models,
            // Theme & Language
            commands::config::get_theme,
            commands::config::set_theme,
            commands::config::get_language,
            commands::config::set_language,
            // User config
            commands::config::get_user_config,
            commands::config::save_user_config,
            commands::config::save_avatar,
            commands::config::get_system_timezone,
            // Tool timeout
            commands::config::get_tool_timeout,
            commands::config::set_tool_timeout,
            // Autostart
            commands::config::get_autostart_enabled,
            commands::config::set_autostart_enabled,
            // Permissions
            permissions::check_all_permissions,
            permissions::check_permission,
            permissions::request_permission,
            // Session management
            commands::session::create_session_cmd,
            commands::session::list_sessions_cmd,
            commands::session::load_session_messages_cmd,
            commands::session::load_session_messages_latest_cmd,
            commands::session::load_session_messages_before_cmd,
            commands::session::get_session_cmd,
            commands::session::delete_session_cmd,
            commands::session::rename_session_cmd,
            commands::session::mark_session_read_cmd,
            commands::session::mark_session_read_batch_cmd,
            commands::session::mark_all_sessions_read_cmd,
            // Window theme
            commands::misc::set_window_theme,
            // Logging
            commands::logging::query_logs_cmd,
            commands::logging::get_log_stats_cmd,
            commands::logging::clear_logs_cmd,
            commands::logging::get_log_config_cmd,
            commands::logging::save_log_config_cmd,
            commands::logging::export_logs_cmd,
            commands::logging::list_log_files_cmd,
            commands::logging::read_log_file_cmd,
            commands::logging::get_log_file_path_cmd,
            commands::logging::frontend_log,
            commands::logging::frontend_log_batch,
            // Cron management
            commands::cron::cron_list_jobs,
            commands::cron::cron_get_job,
            commands::cron::cron_create_job,
            commands::cron::cron_update_job,
            commands::cron::cron_delete_job,
            commands::cron::cron_toggle_job,
            commands::cron::cron_run_now,
            commands::cron::cron_get_run_logs,
            commands::cron::cron_get_calendar_events,
            // Sub-agent management
            commands::subagent::list_subagent_runs,
            commands::subagent::get_subagent_run,
            commands::subagent::kill_subagent,
            // Crash recovery & backup
            commands::crash::get_crash_recovery_info,
            commands::crash::get_crash_history,
            commands::crash::clear_crash_history,
            commands::crash::request_app_restart,
            commands::crash::list_backups_cmd,
            commands::crash::restore_backup_cmd,
            commands::crash::create_backup_cmd,
            commands::crash::get_guardian_enabled,
            commands::crash::set_guardian_enabled,
            // Sandbox
            sandbox::get_sandbox_config,
            sandbox::set_sandbox_config,
            sandbox::check_sandbox_available,
            // Slash commands
            slash_commands::list_slash_commands,
            slash_commands::execute_slash_command,
            slash_commands::is_slash_command,
            // Canvas
            tools::canvas::canvas_submit_snapshot,
            tools::canvas::canvas_submit_eval_result,
            tools::canvas::get_canvas_config,
            tools::canvas::save_canvas_config,
            tools::canvas::list_canvas_projects,
            tools::canvas::get_canvas_project,
            tools::canvas::delete_canvas_project,
            tools::canvas::show_canvas_panel,
            // Dashboard analytics
            commands::dashboard::dashboard_overview,
            commands::dashboard::dashboard_token_usage,
            commands::dashboard::dashboard_tool_usage,
            commands::dashboard::dashboard_sessions,
            commands::dashboard::dashboard_errors,
            commands::dashboard::dashboard_tasks,
            commands::dashboard::dashboard_system_metrics,
            // Developer tools
            dev_tools::dev_clear_sessions,
            dev_tools::dev_clear_cron,
            dev_tools::dev_clear_memory,
            dev_tools::dev_reset_config,
            dev_tools::dev_clear_all,
            // Plan mode
            commands::plan::get_plan_mode,
            commands::plan::set_plan_mode,
            commands::plan::get_plan_content,
            commands::plan::save_plan_content,
            commands::plan::get_plan_steps,
            commands::plan::update_plan_step_status,
            // ACP control plane
            commands::acp_control::acp_list_backends,
            commands::acp_control::acp_health_check,
            commands::acp_control::acp_refresh_backends,
            commands::acp_control::acp_list_runs,
            commands::acp_control::acp_kill_run,
            commands::acp_control::acp_get_run_result,
            commands::acp_control::acp_get_config,
            commands::acp_control::acp_set_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
