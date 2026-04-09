// OpenComputer HTTP/WebSocket Server
// Depends on oc-core for business logic, uses axum 0.8 for HTTP.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use axum::routing::{delete, get, patch, post, put};
use axum::Router;
use tower_http::cors::{AllowOrigin, CorsLayer};

use oc_core::event_bus::EventBus;
use oc_core::session::SessionDB;

pub mod config;
pub mod error;
pub mod middleware;
pub mod routes;
pub mod ws;

pub use config::ServerConfig;

// ── AppContext ───────────────────────────────────────────────────

/// Shared application state passed to all handlers via `State<Arc<AppContext>>`.
pub struct AppContext {
    pub session_db: Arc<SessionDB>,
    pub event_bus: Arc<dyn EventBus>,
    /// Per-session broadcast channels for chat streaming via WebSocket.
    pub chat_streams: Arc<ws::chat_stream::ChatStreamRegistry>,
    /// Per-session cancel flags. Key = session_id.
    pub chat_cancels: Arc<RwLock<HashMap<String, Arc<AtomicBool>>>>,
}

// ── Router Builder ──────────────────────────────────────────────

/// Build the full axum `Router` with all API routes and WebSocket endpoints.
/// Uses permissive CORS (allow all origins), no API key auth.
pub fn build_router(ctx: Arc<AppContext>) -> Router {
    build_router_with_cors(ctx, &[], None)
}

/// Start the HTTP/WebSocket server, binding to the configured address.
pub async fn start_server(config: ServerConfig, ctx: Arc<AppContext>) -> anyhow::Result<()> {
    let router = build_router_with_cors(ctx, &config.cors_origins, config.api_key.clone());

    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await?;
    eprintln!("[oc-server] listening on {}", config.bind_addr);

    axum::serve(listener, router).await?;
    Ok(())
}

// ── Internal Helpers ────────────────────────────────────────────

/// Build the router with specific CORS origins and optional API key auth.
fn build_router_with_cors(
    ctx: Arc<AppContext>,
    cors_origins: &[String],
    api_key: Option<String>,
) -> Router {
    // Health endpoint is always public (no auth required)
    let health = Router::new().route("/api/health", get(routes::health::health_check));

    // Protected API routes
    let api = Router::new()
        // Sessions
        .route("/sessions", post(routes::sessions::create_session))
        .route("/sessions", get(routes::sessions::list_sessions))
        .route("/sessions/{id}", get(routes::sessions::get_session))
        .route("/sessions/{id}", delete(routes::sessions::delete_session))
        .route("/sessions/{id}", patch(routes::sessions::rename_session))
        .route(
            "/sessions/{id}/messages",
            get(routes::sessions::get_session_messages),
        )
        // Chat
        .route("/chat", post(routes::chat::chat))
        .route("/chat/stop", post(routes::chat::stop_chat))
        .route(
            "/chat/approval/{request_id}",
            post(routes::chat::respond_to_approval),
        )
        .route("/chat/system-prompt", get(routes::chat::get_system_prompt))
        .route("/chat/tools", get(routes::chat::list_tools))
        // Providers
        .route("/providers", get(routes::providers::list_providers))
        .route("/providers", post(routes::providers::add_provider))
        .route("/providers/{id}", put(routes::providers::update_provider))
        .route(
            "/providers/{id}",
            delete(routes::providers::delete_provider),
        )
        .route("/providers/test", post(routes::providers::test_provider))
        .route(
            "/providers/active-model",
            get(routes::providers::get_active_model),
        )
        .route(
            "/providers/active-model",
            put(routes::providers::set_active_model),
        )
        // Memory
        .route("/memory", post(routes::memory::add_memory))
        .route("/memory", get(routes::memory::list_memories))
        .route("/memory/{id}", get(routes::memory::get_memory))
        .route("/memory/{id}", put(routes::memory::update_memory))
        .route("/memory/{id}", delete(routes::memory::delete_memory))
        .route("/memory/search", post(routes::memory::search_memories))
        .route("/memory/count", get(routes::memory::memory_count))
        .route("/memory/stats", get(routes::memory::memory_stats))
        // Config
        .route("/config/user", get(routes::config::get_user_config))
        .route("/config/user", put(routes::config::save_user_config))
        .route("/config/web-search", get(routes::config::get_web_search_config))
        .route("/config/web-search", put(routes::config::save_web_search_config))
        .route("/config/proxy", get(routes::config::get_proxy_config))
        .route("/config/proxy", put(routes::config::save_proxy_config))
        .route("/config/compact", get(routes::config::get_compact_config))
        .route("/config/compact", put(routes::config::save_compact_config))
        .route("/config/notification", get(routes::config::get_notification_config))
        .route("/config/notification", put(routes::config::save_notification_config))
        .route("/config/plan-subagent", get(routes::config::get_plan_subagent))
        .route("/config/plan-subagent", post(routes::config::set_plan_subagent))
        .route("/config/plan-question-timeout", get(routes::config::get_plan_question_timeout))
        .route("/config/plan-question-timeout", post(routes::config::set_plan_question_timeout))
        .route("/config/server", get(routes::config::get_server_config))
        .route("/config/server", put(routes::config::save_server_config))
        // Agents
        .route("/agents", get(routes::agents::list_agents))
        .route("/agents/{id}", get(routes::agents::get_agent))
        .route("/agents/{id}", put(routes::agents::save_agent))
        .route("/agents/{id}", delete(routes::agents::delete_agent))
        // Cron
        .route("/cron/jobs", get(routes::cron::list_jobs))
        .route("/cron/jobs", post(routes::cron::create_job))
        .route("/cron/jobs/{id}", get(routes::cron::get_job))
        .route("/cron/jobs/{id}", put(routes::cron::update_job))
        .route("/cron/jobs/{id}", delete(routes::cron::delete_job))
        .route("/cron/jobs/{id}/toggle", post(routes::cron::toggle_job))
        .route("/cron/jobs/{id}/run", post(routes::cron::run_now))
        .route("/cron/jobs/{id}/logs", get(routes::cron::get_run_logs))
        .route("/cron/calendar", get(routes::cron::get_calendar_events))
        // Dashboard
        .route("/dashboard/overview", post(routes::dashboard::overview))
        .route("/dashboard/token-usage", post(routes::dashboard::token_usage))
        .route("/dashboard/tool-usage", post(routes::dashboard::tool_usage))
        .route("/dashboard/sessions", post(routes::dashboard::sessions))
        .route("/dashboard/errors", post(routes::dashboard::errors))
        .route("/dashboard/tasks", post(routes::dashboard::tasks))
        .route("/dashboard/system-metrics", get(routes::dashboard::system_metrics))
        .route("/dashboard/session-list", post(routes::dashboard::session_list))
        .route("/dashboard/message-list", post(routes::dashboard::message_list))
        .route("/dashboard/tool-call-list", post(routes::dashboard::tool_call_list))
        .route("/dashboard/error-list", post(routes::dashboard::error_list))
        .route("/dashboard/agent-list", post(routes::dashboard::agent_list))
        // Plan Mode
        .route("/plan/{sid}/mode", get(routes::plan::get_plan_mode))
        .route("/plan/{sid}/mode", post(routes::plan::set_plan_mode))
        .route("/plan/{sid}/content", get(routes::plan::get_plan_content))
        .route("/plan/{sid}/content", put(routes::plan::save_plan_content))
        .route("/plan/{sid}/steps", get(routes::plan::get_plan_steps))
        .route("/plan/{sid}/steps/update", post(routes::plan::update_plan_step_status))
        .route("/plan/question-response", post(routes::plan::respond_plan_question))
        .route("/plan/{sid}/versions", get(routes::plan::get_plan_versions))
        .route("/plan/version/load", post(routes::plan::load_plan_version_content))
        .route("/plan/{sid}/version/restore", post(routes::plan::restore_plan_version))
        .route("/plan/{sid}/rollback", post(routes::plan::plan_rollback))
        .route("/plan/{sid}/checkpoint", get(routes::plan::get_plan_checkpoint))
        .route("/plan/{sid}/file-path", get(routes::plan::get_plan_file_path))
        .route("/plan/{sid}/cancel", post(routes::plan::cancel_plan_subagent))
        // Logging
        .route("/logs/query", post(routes::logging::query_logs))
        .route("/logs/stats", get(routes::logging::get_log_stats))
        .route("/logs/clear", post(routes::logging::clear_logs))
        .route("/logs/config", get(routes::logging::get_log_config))
        .route("/logs/config", put(routes::logging::save_log_config))
        .route("/logs/files", get(routes::logging::list_log_files))
        .route("/logs/file", get(routes::logging::read_log_file))
        .route("/logs/file-path", get(routes::logging::get_log_file_path))
        .route("/logs/frontend", post(routes::logging::frontend_log))
        .route("/logs/frontend-batch", post(routes::logging::frontend_log_batch))
        .route("/logs/export", post(routes::logging::export_logs))
        // Skills
        .route("/skills", get(routes::skills::list_skills))
        .route("/skills/env-check", get(routes::skills::get_skill_env_check))
        .route("/skills/env-check", put(routes::skills::set_skill_env_check))
        .route("/skills/env-status", get(routes::skills::get_skills_env_status))
        .route("/skills/status", get(routes::skills::get_skills_status))
        .route("/skills/extra-dirs", get(routes::skills::get_extra_skills_dirs))
        .route("/skills/extra-dirs", post(routes::skills::add_extra_skills_dir))
        .route("/skills/extra-dirs", delete(routes::skills::remove_extra_skills_dir))
        .route("/skills/{name}", get(routes::skills::get_skill_detail))
        .route("/skills/{name}/toggle", post(routes::skills::toggle_skill))
        .route("/skills/{name}/env", get(routes::skills::get_skill_env))
        .route("/skills/{name}/env", post(routes::skills::set_skill_env_var))
        .route("/skills/{name}/env", delete(routes::skills::remove_skill_env_var))
        // Channel
        .route("/channel/plugins", get(routes::channel::list_plugins))
        .route("/channel/accounts", get(routes::channel::list_accounts))
        .route("/channel/accounts", post(routes::channel::add_account))
        .route("/channel/accounts/{id}", put(routes::channel::update_account))
        .route("/channel/accounts/{id}", delete(routes::channel::remove_account))
        .route("/channel/accounts/{id}/start", post(routes::channel::start_account))
        .route("/channel/accounts/{id}/stop", post(routes::channel::stop_account))
        .route("/channel/accounts/{id}/health", get(routes::channel::health))
        .route("/channel/accounts/{id}/test-message", post(routes::channel::send_test_message))
        .route("/channel/health", get(routes::channel::health_all))
        .route("/channel/validate", post(routes::channel::validate_credentials))
        .route("/channel/sessions", get(routes::channel::list_sessions))
        .route("/channel/wechat/login/start", post(routes::channel::wechat_start_login))
        .route("/channel/wechat/login/wait", post(routes::channel::wechat_wait_login))
        // Crash / Backup
        .route("/crash/recovery-info", get(routes::crash::get_crash_recovery_info))
        .route("/crash/history", get(routes::crash::get_crash_history))
        .route("/crash/history", delete(routes::crash::clear_crash_history))
        .route("/crash/backups", get(routes::crash::list_backups))
        .route("/crash/backups", post(routes::crash::create_backup))
        .route("/crash/backups/restore", post(routes::crash::restore_backup))
        .route("/crash/guardian", get(routes::crash::get_guardian_enabled))
        .route("/crash/guardian", put(routes::crash::set_guardian_enabled))
        // URL Preview
        .route("/url-preview", post(routes::url_preview::fetch_url_preview))
        .route("/url-preview/batch", post(routes::url_preview::fetch_url_previews))
        // Subagent
        .route("/subagent/runs", get(routes::subagent::list_subagent_runs))
        .route("/subagent/runs/{run_id}", get(routes::subagent::get_subagent_run))
        .route("/subagent/runs/{run_id}/kill", post(routes::subagent::kill_subagent))
        // ACP Control
        .route("/acp/backends", get(routes::acp::list_backends))
        .route("/acp/refresh", post(routes::acp::refresh_backends))
        .route("/acp/runs", get(routes::acp::list_runs))
        .route("/acp/runs/{run_id}/kill", post(routes::acp::kill_run))
        .route("/acp/runs/{run_id}/result", get(routes::acp::get_run_result))
        .route("/acp/config", get(routes::acp::get_config))
        .route("/acp/config", put(routes::acp::set_config))
        // Weather
        .route("/weather/geocode", get(routes::weather::geocode_search))
        .route("/weather/preview", post(routes::weather::preview_weather))
        .route("/weather/current", get(routes::weather::get_current_weather))
        .route("/weather/refresh", post(routes::weather::refresh_weather))
        .route("/weather/detect-location", get(routes::weather::detect_location))
        // Slash commands
        .route("/slash-commands", get(routes::slash::list_slash_commands))
        .route("/slash-commands/execute", post(routes::slash::execute_slash_command))
        .route("/slash-commands/is-slash", post(routes::slash::is_slash_command))
        // Canvas
        .route("/canvas/snapshot/{request_id}", post(routes::canvas::canvas_submit_snapshot))
        .route("/canvas/eval/{request_id}", post(routes::canvas::canvas_submit_eval_result))
        // Providers extras
        .route("/providers/available-models", get(routes::providers::get_available_models))
        .route("/providers/reorder", post(routes::providers::reorder_providers))
        // Misc
        .route("/misc/write-export-file", post(routes::misc::write_export_file));

    let ws_routes = Router::new()
        .route("/events", get(ws::events::events_ws))
        .route("/chat/{session_id}", get(ws::chat_stream::chat_stream_ws));

    // Apply API key auth middleware to protected routes
    let auth_state = middleware::ApiKeyState { api_key };
    let protected = Router::new()
        .nest("/api", api)
        .nest("/ws", ws_routes)
        .route_layer(axum::middleware::from_fn_with_state(
            auth_state,
            middleware::require_api_key,
        ));

    Router::new()
        .merge(health)
        .merge(protected)
        .layer(build_cors_layer(cors_origins))
        .with_state(ctx)
}

/// Build a CORS layer. When `origins` is empty, allow all origins (permissive).
fn build_cors_layer(origins: &[String]) -> CorsLayer {
    let cors = CorsLayer::new()
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any);

    if origins.is_empty() {
        cors.allow_origin(AllowOrigin::any())
    } else {
        let parsed: Vec<_> = origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        cors.allow_origin(parsed)
    }
}
