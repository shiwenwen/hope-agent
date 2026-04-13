// ── Local Tauri-specific modules ──────────────────────────────────
mod app_init;
mod commands;
mod globals;
mod setup;
mod shortcuts;
mod tauri_wrappers;
mod tray;

// ── Re-export all business logic from oc-core ────────────────────
// This makes `crate::agent`, `crate::session`, etc. resolve to oc-core's modules,
// eliminating the need for duplicate local copies.

pub use oc_core::acp;
pub use oc_core::acp_control;
pub use oc_core::agent;
pub use oc_core::agent_config;
pub use oc_core::agent_loader;
pub use oc_core::backup;
pub use oc_core::browser_state;
pub use oc_core::canvas_db;
pub use oc_core::channel;
pub use oc_core::chat_engine;
pub use oc_core::context_compact;
pub use oc_core::crash_journal;
pub use oc_core::cron;
pub use oc_core::dashboard;
pub use oc_core::dev_tools;
pub use oc_core::docker;
pub use oc_core::failover;
pub use oc_core::file_extract;
pub use oc_core::guardian;
pub use oc_core::logging;
pub use oc_core::memory;
pub use oc_core::memory_extract;
pub use oc_core::oauth;
pub use oc_core::paths;
pub use oc_core::permissions;
pub use oc_core::plan;
pub use oc_core::process_registry;
pub use oc_core::provider;
pub use oc_core::sandbox;
pub use oc_core::self_diagnosis;
pub use oc_core::service_install;
pub use oc_core::session;
pub use oc_core::skills;
pub use oc_core::slash_commands;
pub use oc_core::subagent;
pub use oc_core::system_prompt;
pub use oc_core::tools;
pub use oc_core::url_preview;
pub use oc_core::user_config;
pub use oc_core::weather;
#[cfg(target_os = "macos")]
pub use oc_core::weather_location_macos;

// Re-export oc-core utility functions (truncate_utf8, default_true, etc.)
pub use oc_core::{default_true, sql_opt_u64, sql_u64, truncate_utf8};

// Re-export oc-core global accessors and types
pub use oc_core::event_bus;
pub use oc_core::init_app_state;
pub use oc_core::{
    get_acp_manager, get_channel_db, get_channel_registry, get_cron_db, get_event_bus, get_logger,
    get_memory_backend, get_session_db, get_subagent_cancels, set_event_bus,
};
pub use oc_core::{
    AppState, ACP_MANAGER, APP_LOGGER, APP_STATE, CHANNEL_DB, CHANNEL_REGISTRY, CRON_DB, EVENT_BUS,
    MEMORY_BACKEND, SESSION_DB, SUBAGENT_CANCELS,
};

// ── Local re-exports ─────────────────────────────────────────────
pub use globals::get_app_handle;
pub(crate) use shortcuts::toggle_quickchat_window;

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

    // Load app config at startup
    let initial_store = oc_core::config::load_config().unwrap_or_default();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // When a second instance is launched, show and focus the existing window
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_process::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(shortcuts::handle_shortcut)
                .build(),
        )
        .on_window_event(|window, event| {
            // Intercept window close → hide instead of quit (app stays resident in tray)
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let label = window.label();
                if label == "main" || label == "quickchat" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(setup::app_setup)
        .manage(app_init::init_tauri_app_state(initial_store))
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
            // System prompt
            commands::chat::get_system_prompt,
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
            commands::memory::memory_get_import_from_ai_prompt,
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
            commands::memory::get_memory_selection_config,
            commands::memory::save_memory_selection_config,
            commands::memory::get_dedup_config,
            commands::memory::save_dedup_config,
            commands::memory::get_hybrid_search_config,
            commands::memory::save_hybrid_search_config,
            commands::memory::get_temporal_decay_config,
            commands::memory::save_temporal_decay_config,
            commands::memory::get_mmr_config,
            commands::memory::save_mmr_config,
            commands::memory::get_embedding_cache_config,
            commands::memory::save_embedding_cache_config,
            commands::memory::get_multimodal_config,
            commands::memory::save_multimodal_config,
            commands::memory::get_embedding_config,
            commands::memory::save_embedding_config,
            commands::memory::get_embedding_presets,
            commands::config::get_compact_config,
            commands::config::save_compact_config,
            commands::config::get_notification_config,
            commands::config::save_notification_config,
            commands::config::get_server_config,
            commands::config::save_server_config,
            commands::config::compact_context_now,
            commands::memory::list_local_embedding_models,
            // Theme & Language
            commands::config::get_theme,
            commands::config::set_theme,
            commands::config::get_language,
            commands::config::set_language,
            commands::config::get_ui_effects_enabled,
            commands::config::set_ui_effects_enabled,
            // User config
            commands::config::get_user_config,
            commands::config::save_user_config,
            commands::config::save_avatar,
            commands::config::get_system_timezone,
            // Tool timeout
            commands::config::get_tool_timeout,
            commands::config::set_tool_timeout,
            commands::config::get_approval_timeout,
            commands::config::set_approval_timeout,
            commands::config::get_approval_timeout_action,
            commands::config::set_approval_timeout_action,
            // Tool result disk persistence
            commands::config::get_tool_result_disk_threshold,
            commands::config::set_tool_result_disk_threshold,
            // Tool limits (image/pdf)
            commands::config::get_tool_limits,
            commands::config::set_tool_limits,
            // Temperature
            commands::config::get_global_temperature,
            commands::config::set_global_temperature,
            commands::config::get_plan_subagent,
            commands::config::set_plan_subagent,
            commands::config::get_ask_user_question_timeout,
            commands::config::set_ask_user_question_timeout,
            // Deferred tool loading
            commands::config::get_deferred_tools_config,
            commands::config::save_deferred_tools_config,
            // Shortcuts
            commands::config::get_shortcut_config,
            commands::config::save_shortcut_config,
            commands::config::set_shortcuts_paused,
            // Weather
            commands::config::geocode_search,
            commands::config::preview_weather,
            commands::config::get_current_weather,
            commands::config::refresh_weather,
            commands::config::detect_location,
            // Autostart
            commands::config::get_autostart_enabled,
            commands::config::set_autostart_enabled,
            // Permissions (thin wrappers over oc-core)
            tauri_wrappers::check_all_permissions,
            tauri_wrappers::check_permission,
            tauri_wrappers::request_permission,
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
            // Sandbox (thin wrappers over oc-core)
            tauri_wrappers::get_sandbox_config,
            tauri_wrappers::set_sandbox_config,
            tauri_wrappers::check_sandbox_available,
            // Slash commands (thin wrappers over oc-core)
            tauri_wrappers::list_slash_commands,
            tauri_wrappers::execute_slash_command,
            tauri_wrappers::is_slash_command,
            // Canvas (thin wrappers over oc-core)
            tauri_wrappers::canvas_submit_snapshot,
            tauri_wrappers::canvas_submit_eval_result,
            tauri_wrappers::get_canvas_config,
            tauri_wrappers::save_canvas_config,
            tauri_wrappers::list_canvas_projects,
            tauri_wrappers::get_canvas_project,
            tauri_wrappers::delete_canvas_project,
            tauri_wrappers::show_canvas_panel,
            // Dashboard analytics
            commands::dashboard::dashboard_overview,
            commands::dashboard::dashboard_token_usage,
            commands::dashboard::dashboard_tool_usage,
            commands::dashboard::dashboard_sessions,
            commands::dashboard::dashboard_errors,
            commands::dashboard::dashboard_tasks,
            commands::dashboard::dashboard_system_metrics,
            commands::dashboard::dashboard_session_list,
            commands::dashboard::dashboard_message_list,
            commands::dashboard::dashboard_tool_call_list,
            commands::dashboard::dashboard_error_list,
            commands::dashboard::dashboard_agent_list,
            commands::dashboard::dashboard_overview_delta,
            commands::dashboard::dashboard_insights,
            // Recap (deep analysis reports)
            commands::recap::recap_generate,
            commands::recap::recap_list_reports,
            commands::recap::recap_get_report,
            commands::recap::recap_delete_report,
            commands::recap::recap_export_html,
            // Developer tools (thin wrappers over oc-core)
            tauri_wrappers::dev_clear_sessions,
            tauri_wrappers::dev_clear_cron,
            tauri_wrappers::dev_clear_memory,
            tauri_wrappers::dev_reset_config,
            tauri_wrappers::dev_clear_all,
            // Plan mode
            commands::plan::get_plan_mode,
            commands::plan::set_plan_mode,
            commands::plan::get_plan_content,
            commands::plan::save_plan_content,
            commands::plan::get_plan_steps,
            commands::plan::update_plan_step_status,
            commands::plan::respond_ask_user_question,
            commands::plan::get_pending_ask_user_group,
            commands::plan::get_plan_versions,
            commands::plan::load_plan_version_content,
            commands::plan::restore_plan_version,
            commands::plan::plan_rollback,
            commands::plan::get_plan_checkpoint,
            commands::plan::get_plan_file_path,
            commands::plan::cancel_plan_subagent,
            // ACP control plane
            commands::acp_control::acp_list_backends,
            commands::acp_control::acp_health_check,
            commands::acp_control::acp_refresh_backends,
            commands::acp_control::acp_list_runs,
            commands::acp_control::acp_kill_run,
            commands::acp_control::acp_get_run_result,
            commands::acp_control::acp_get_config,
            commands::acp_control::acp_set_config,
            // URL preview
            commands::url_preview::fetch_url_preview,
            commands::url_preview::fetch_url_previews,
            // IM Channel management
            commands::channel::channel_list_plugins,
            commands::channel::channel_list_accounts,
            commands::channel::channel_add_account,
            commands::channel::channel_update_account,
            commands::channel::channel_remove_account,
            commands::channel::channel_start_account,
            commands::channel::channel_stop_account,
            commands::channel::channel_health,
            commands::channel::channel_health_all,
            commands::channel::channel_validate_credentials,
            commands::channel::channel_send_test_message,
            commands::channel::channel_list_sessions,
            commands::channel::channel_wechat_start_login,
            commands::channel::channel_wechat_wait_login,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // macOS: clicking Dock icon when all windows are hidden → show main window
            if let tauri::RunEvent::Reopen { .. } = event {
                use tauri::Manager;
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
        });
}
