// OpenComputer HTTP/WebSocket Server
// Depends on oc-core for business logic, uses axum 0.8 for HTTP.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, patch, post, put};
use axum::Router;
use tower_http::cors::{AllowOrigin, CorsLayer};

use oc_core::event_bus::EventBus;
use oc_core::project::ProjectDB;
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
    pub project_db: Arc<ProjectDB>,
    pub event_bus: Arc<dyn EventBus>,
    /// Per-session broadcast channels for chat streaming via WebSocket.
    pub chat_streams: Arc<ws::chat_stream::ChatStreamRegistry>,
    /// Per-session cancel flags. Key = session_id.
    pub chat_cancels: Arc<RwLock<HashMap<String, Arc<AtomicBool>>>>,
    /// API key used by middleware auth, reused by attachment URL rewrite to
    /// stamp `?token=` onto `/api/attachments/*` URLs emitted in events.
    /// `None` when server runs in no-auth mode.
    pub api_key: Option<String>,
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
        .route(
            "/sessions/{id}/read",
            post(routes::sessions::mark_session_read),
        )
        .route(
            "/sessions/read-batch",
            post(routes::sessions::mark_session_read_batch),
        )
        .route(
            "/sessions/read-all",
            post(routes::sessions::mark_all_sessions_read),
        )
        .route(
            "/sessions/{id}/compact",
            post(routes::sessions::compact_context_now),
        )
        .route(
            "/sessions/{id}/project",
            patch(routes::projects::move_session_to_project),
        )
        .route(
            "/sessions/{id}/awareness-config",
            get(routes::sessions::get_session_awareness_config),
        )
        .route(
            "/sessions/{id}/awareness-config",
            patch(routes::sessions::set_session_awareness_config),
        )
        .route("/sessions/search", get(routes::sessions::search_sessions))
        // Projects
        .route("/projects", get(routes::projects::list_projects))
        .route("/projects", post(routes::projects::create_project))
        .route("/projects/{id}", get(routes::projects::get_project))
        .route("/projects/{id}", patch(routes::projects::update_project))
        .route("/projects/{id}", delete(routes::projects::delete_project))
        .route(
            "/projects/{id}/archive",
            post(routes::projects::archive_project),
        )
        .route(
            "/projects/{id}/sessions",
            get(routes::projects::list_project_sessions),
        )
        .route(
            "/projects/{id}/files",
            get(routes::projects::list_project_files),
        )
        .route(
            "/projects/{id}/files",
            post(routes::projects::upload_project_file_route)
                .layer(DefaultBodyLimit::max(25 * 1024 * 1024)),
        )
        .route(
            "/projects/{id}/files/{fid}",
            delete(routes::projects::delete_project_file_route),
        )
        .route(
            "/projects/{id}/files/{fid}",
            patch(routes::projects::rename_project_file_route),
        )
        .route(
            "/projects/{id}/files/{fid}/content",
            get(routes::projects::read_project_file_content),
        )
        .route(
            "/projects/{id}/memories",
            get(routes::projects::list_project_memories),
        )
        .route(
            "/sessions/{id}/messages/around",
            get(routes::sessions::get_session_messages_around),
        )
        .route(
            "/sessions/{id}/messages/search",
            get(routes::sessions::search_session_messages),
        )
        .route(
            "/sessions/{id}/stream-state",
            get(routes::sessions::get_session_stream_state),
        )
        // Chat
        .route("/chat", post(routes::chat::chat))
        .route("/chat/stop", post(routes::chat::stop_chat))
        .route(
            "/chat/approval/{request_id}",
            post(routes::chat::respond_to_approval),
        )
        .route(
            "/chat/approval",
            post(routes::chat::respond_to_approval_body),
        )
        .route(
            "/chat/attachment",
            post(routes::chat::save_attachment)
                .layer(DefaultBodyLimit::max(25 * 1024 * 1024)),
        )
        // Attachment download (serves session-scoped files under
        // ~/.opencomputer/attachments/{session_id}/) — the logical URL
        // form emitted in `__MEDIA_ITEMS__` events.
        .route(
            "/attachments/{session_id}/{filename}",
            get(routes::attachments::download),
        )
        .route("/chat/system-prompt", get(routes::chat::get_system_prompt))
        .route("/system-prompt", post(routes::chat::get_system_prompt_post))
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
            "/providers/test-embedding",
            post(routes::providers::test_embedding),
        )
        .route(
            "/providers/test-image",
            post(routes::providers::test_image_generate),
        )
        .route(
            "/providers/active-model",
            get(routes::providers::get_active_model),
        )
        .route(
            "/providers/active-model",
            put(routes::providers::set_active_model),
        )
        // Models (aliases under /api/models/*)
        .route("/models", get(routes::models::list_available_models))
        .route("/models/active", get(routes::models::get_active_model))
        .route("/models/active", post(routes::models::set_active_model))
        .route("/models/fallback", get(routes::models::get_fallback_models))
        .route("/models/fallback", post(routes::models::set_fallback_models))
        .route(
            "/models/reasoning-effort",
            post(routes::models::set_reasoning_effort),
        )
        .route("/models/settings", get(routes::models::get_current_settings))
        .route(
            "/models/temperature",
            get(routes::models::get_global_temperature),
        )
        .route(
            "/models/temperature",
            post(routes::models::set_global_temperature),
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
        .route(
            "/memory/import-from-ai-prompt",
            get(routes::memory::import_from_ai_prompt),
        )
        .route("/memory/{id}/pin", post(routes::memory::toggle_pin))
        .route("/memory/delete-batch", post(routes::memory::delete_batch))
        .route("/memory/reembed", post(routes::memory::reembed))
        .route(
            "/memory/global-md",
            get(routes::memory::get_global_memory_md),
        )
        .route(
            "/memory/global-md",
            put(routes::memory::save_global_memory_md),
        )
        // Config
        .route("/config/user", get(routes::config::get_user_config))
        .route("/config/user", put(routes::config::save_user_config))
        .route(
            "/config/web-search",
            get(routes::config::get_web_search_config),
        )
        .route(
            "/config/web-search",
            put(routes::config::save_web_search_config),
        )
        .route("/config/proxy", get(routes::config::get_proxy_config))
        .route("/config/proxy", put(routes::config::save_proxy_config))
        .route("/config/compact", get(routes::config::get_compact_config))
        .route("/config/compact", put(routes::config::save_compact_config))
        .route(
            "/config/awareness",
            get(routes::config::get_awareness_config),
        )
        .route(
            "/config/awareness",
            put(routes::config::save_awareness_config),
        )
        .route("/config/recap", get(routes::config::get_recap_config))
        .route("/config/recap", put(routes::config::save_recap_config))
        .route(
            "/config/async-tools",
            get(routes::config::get_async_tools_config),
        )
        .route(
            "/config/async-tools",
            put(routes::config::save_async_tools_config),
        )
        .route(
            "/config/deferred-tools",
            get(routes::config::get_deferred_tools_config),
        )
        .route(
            "/config/deferred-tools",
            put(routes::config::save_deferred_tools_config),
        )
        .route(
            "/config/memory-selection",
            get(routes::config::get_memory_selection_config),
        )
        .route(
            "/config/memory-selection",
            put(routes::config::save_memory_selection_config),
        )
        .route(
            "/config/memory-budget",
            get(routes::config::get_memory_budget_config),
        )
        .route(
            "/config/memory-budget",
            put(routes::config::save_memory_budget_config),
        )
        .route(
            "/config/notification",
            get(routes::config::get_notification_config),
        )
        .route(
            "/config/notification",
            put(routes::config::save_notification_config),
        )
        .route(
            "/config/tool-timeout",
            get(routes::config::get_tool_timeout),
        )
        .route(
            "/config/tool-timeout",
            post(routes::config::set_tool_timeout),
        )
        .route(
            "/config/approval-timeout",
            get(routes::config::get_approval_timeout),
        )
        .route(
            "/config/approval-timeout",
            post(routes::config::set_approval_timeout),
        )
        .route(
            "/config/approval-timeout-action",
            get(routes::config::get_approval_timeout_action),
        )
        .route(
            "/config/approval-timeout-action",
            post(routes::config::set_approval_timeout_action),
        )
        .route(
            "/config/tool-result-threshold",
            get(routes::config::get_tool_result_disk_threshold),
        )
        .route(
            "/config/tool-result-threshold",
            post(routes::config::set_tool_result_disk_threshold),
        )
        .route("/config/tool-limits", get(routes::config::get_tool_limits))
        .route("/config/tool-limits", post(routes::config::set_tool_limits))
        .route(
            "/config/plan-subagent",
            get(routes::config::get_plan_subagent),
        )
        .route(
            "/config/plan-subagent",
            post(routes::config::set_plan_subagent),
        )
        .route(
            "/config/ask-user-question-timeout",
            get(routes::config::get_ask_user_question_timeout),
        )
        .route(
            "/config/ask-user-question-timeout",
            post(routes::config::set_ask_user_question_timeout),
        )
        .route("/config/server", get(routes::config::get_server_config))
        .route("/config/server", put(routes::config::save_server_config))
        // Config — memory
        .route(
            "/config/embedding",
            get(routes::config::get_embedding_config),
        )
        .route(
            "/config/embedding",
            put(routes::config::save_embedding_config),
        )
        .route(
            "/config/embedding/presets",
            get(routes::config::get_embedding_presets),
        )
        .route(
            "/config/embedding-cache",
            get(routes::config::get_embedding_cache_config),
        )
        .route(
            "/config/embedding-cache",
            put(routes::config::save_embedding_cache_config),
        )
        .route("/config/dedup", get(routes::config::get_dedup_config))
        .route("/config/dedup", put(routes::config::save_dedup_config))
        .route(
            "/config/hybrid-search",
            get(routes::config::get_hybrid_search_config),
        )
        .route(
            "/config/hybrid-search",
            put(routes::config::save_hybrid_search_config),
        )
        .route("/config/mmr", get(routes::config::get_mmr_config))
        .route("/config/mmr", put(routes::config::save_mmr_config))
        .route(
            "/config/multimodal",
            get(routes::config::get_multimodal_config),
        )
        .route(
            "/config/multimodal",
            put(routes::config::save_multimodal_config),
        )
        .route(
            "/config/temporal-decay",
            get(routes::config::get_temporal_decay_config),
        )
        .route(
            "/config/temporal-decay",
            put(routes::config::save_temporal_decay_config),
        )
        .route("/config/extract", get(routes::config::get_extract_config))
        .route("/config/extract", put(routes::config::save_extract_config))
        // Config — tools
        .route(
            "/config/web-fetch",
            get(routes::config::get_web_fetch_config),
        )
        .route(
            "/config/web-fetch",
            put(routes::config::save_web_fetch_config),
        )
        .route("/config/ssrf", get(routes::config::get_ssrf_config))
        .route("/config/ssrf", put(routes::config::save_ssrf_config))
        .route(
            "/config/image-generate",
            get(routes::config::get_image_generate_config),
        )
        .route(
            "/config/image-generate",
            put(routes::config::save_image_generate_config),
        )
        .route("/config/canvas", get(routes::config::get_canvas_config))
        .route("/config/canvas", put(routes::config::save_canvas_config))
        .route("/config/sandbox", get(routes::config::get_sandbox_config))
        .route("/config/sandbox", put(routes::config::set_sandbox_config))
        // Config — shortcuts
        .route(
            "/config/shortcuts",
            get(routes::config::get_shortcut_config),
        )
        .route(
            "/config/shortcuts",
            put(routes::config::save_shortcut_config),
        )
        .route(
            "/config/shortcuts/pause",
            post(routes::config::set_shortcuts_paused),
        )
        // Config — theme / language / UI
        .route("/config/theme", get(routes::config::get_theme))
        .route("/config/theme", post(routes::config::set_theme))
        .route("/config/window-theme", post(routes::config::set_window_theme))
        .route("/config/language", get(routes::config::get_language))
        .route("/config/language", post(routes::config::set_language))
        .route(
            "/config/ui-effects",
            get(routes::config::get_ui_effects_enabled),
        )
        .route(
            "/config/ui-effects",
            post(routes::config::set_ui_effects_enabled),
        )
        .route(
            "/config/tool-call-narration",
            get(routes::config::get_tool_call_narration_enabled),
        )
        .route(
            "/config/tool-call-narration",
            post(routes::config::set_tool_call_narration_enabled),
        )
        .route(
            "/config/autostart",
            get(routes::config::get_autostart_enabled),
        )
        .route(
            "/config/autostart",
            post(routes::config::set_autostart_enabled),
        )
        // Agents
        .route("/agents", get(routes::agents::list_agents))
        .route("/agents/template", get(routes::agents::get_agent_template))
        .route(
            "/agents/openclaw/scan",
            get(routes::agents::scan_openclaw_agents),
        )
        .route(
            "/agents/openclaw/import",
            post(routes::agents::import_openclaw_agents),
        )
        .route("/agents/{id}", get(routes::agents::get_agent))
        .route("/agents/{id}", put(routes::agents::save_agent))
        .route("/agents/{id}", delete(routes::agents::delete_agent))
        .route(
            "/agents/{id}/markdown",
            get(routes::agents::get_agent_markdown),
        )
        .route(
            "/agents/{id}/markdown",
            put(routes::agents::save_agent_markdown),
        )
        .route(
            "/agents/{id}/persona/render-soul-md",
            axum::routing::post(routes::agents::render_persona_to_soul_md),
        )
        .route(
            "/agents/{id}/memory-md",
            get(routes::agents::get_agent_memory_md),
        )
        .route(
            "/agents/{id}/memory-md",
            put(routes::agents::save_agent_memory_md),
        )
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
        // Dreaming (offline memory consolidation, Phase B3)
        .route("/dreaming/run", post(routes::dreaming::run_now))
        .route("/dreaming/diaries", get(routes::dreaming::list_diaries))
        .route(
            "/dreaming/diaries/{filename}",
            get(routes::dreaming::read_diary),
        )
        .route("/dreaming/status", get(routes::dreaming::status))
        // Dashboard
        .route("/dashboard/overview", post(routes::dashboard::overview))
        .route(
            "/dashboard/token-usage",
            post(routes::dashboard::token_usage),
        )
        .route("/dashboard/tool-usage", post(routes::dashboard::tool_usage))
        .route("/dashboard/sessions", post(routes::dashboard::sessions))
        .route("/dashboard/errors", post(routes::dashboard::errors))
        .route("/dashboard/tasks", post(routes::dashboard::tasks))
        .route(
            "/dashboard/system-metrics",
            get(routes::dashboard::system_metrics),
        )
        .route(
            "/dashboard/session-list",
            post(routes::dashboard::session_list),
        )
        .route(
            "/dashboard/message-list",
            post(routes::dashboard::message_list),
        )
        .route(
            "/dashboard/tool-call-list",
            post(routes::dashboard::tool_call_list),
        )
        .route("/dashboard/error-list", post(routes::dashboard::error_list))
        .route("/dashboard/agent-list", post(routes::dashboard::agent_list))
        .route(
            "/dashboard/overview-delta",
            post(routes::dashboard::overview_delta),
        )
        .route("/dashboard/insights", post(routes::dashboard::insights))
        .route(
            "/dashboard/learning/overview",
            post(routes::dashboard::learning_overview),
        )
        .route(
            "/dashboard/learning/timeline",
            post(routes::dashboard::learning_timeline),
        )
        .route(
            "/dashboard/learning/top-skills",
            post(routes::dashboard::top_skills),
        )
        .route(
            "/dashboard/learning/recall-stats",
            post(routes::dashboard::recall_stats),
        )
        // Recap
        .route("/recap/generate", post(routes::recap::generate))
        .route("/recap/reports", post(routes::recap::list_reports))
        .route("/recap/reports/{id}", get(routes::recap::get_report))
        .route("/recap/reports/{id}", delete(routes::recap::delete_report))
        .route(
            "/recap/reports/{id}/export",
            post(routes::recap::export_html),
        )
        // Plan Mode
        .route("/plan/{sid}/mode", get(routes::plan::get_plan_mode))
        .route("/plan/{sid}/mode", post(routes::plan::set_plan_mode))
        .route("/plan/{sid}/content", get(routes::plan::get_plan_content))
        .route("/plan/{sid}/content", put(routes::plan::save_plan_content))
        .route("/plan/{sid}/steps", get(routes::plan::get_plan_steps))
        .route(
            "/plan/{sid}/steps/update",
            post(routes::plan::update_plan_step_status),
        )
        .route(
            "/ask_user/respond",
            post(routes::plan::respond_ask_user_question),
        )
        .route(
            "/plan/{sid}/pending-ask-user",
            get(routes::plan::get_pending_ask_user_group),
        )
        .route("/plan/{sid}/versions", get(routes::plan::get_plan_versions))
        .route(
            "/plan/version/load",
            post(routes::plan::load_plan_version_content),
        )
        .route(
            "/plan/{sid}/version/restore",
            post(routes::plan::restore_plan_version),
        )
        .route("/plan/{sid}/rollback", post(routes::plan::plan_rollback))
        .route(
            "/plan/{sid}/checkpoint",
            get(routes::plan::get_plan_checkpoint),
        )
        .route(
            "/plan/{sid}/file-path",
            get(routes::plan::get_plan_file_path),
        )
        .route(
            "/plan/{sid}/cancel",
            post(routes::plan::cancel_plan_subagent),
        )
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
        .route(
            "/logs/frontend-batch",
            post(routes::logging::frontend_log_batch),
        )
        .route("/logs/export", post(routes::logging::export_logs))
        // Skills
        .route("/skills", get(routes::skills::list_skills))
        .route(
            "/skills/env-check",
            get(routes::skills::get_skill_env_check),
        )
        .route(
            "/skills/env-check",
            put(routes::skills::set_skill_env_check),
        )
        .route(
            "/skills/env-status",
            get(routes::skills::get_skills_env_status),
        )
        .route("/skills/status", get(routes::skills::get_skills_status))
        .route("/skills/drafts", get(routes::skills::list_draft_skills))
        .route(
            "/skills/review/run",
            post(routes::skills::trigger_skill_review_now),
        )
        .route(
            "/skills/{name}/activate",
            post(routes::skills::activate_draft_skill),
        )
        .route(
            "/skills/{name}/draft",
            delete(routes::skills::discard_draft_skill),
        )
        .route(
            "/skills/extra-dirs",
            get(routes::skills::get_extra_skills_dirs),
        )
        .route(
            "/skills/extra-dirs",
            post(routes::skills::add_extra_skills_dir),
        )
        .route(
            "/skills/extra-dirs",
            delete(routes::skills::remove_extra_skills_dir),
        )
        .route("/skills/{name}", get(routes::skills::get_skill_detail))
        .route("/skills/{name}/toggle", post(routes::skills::toggle_skill))
        .route("/skills/{name}/env", get(routes::skills::get_skill_env))
        .route(
            "/skills/{name}/env",
            post(routes::skills::set_skill_env_var),
        )
        .route(
            "/skills/{name}/env",
            delete(routes::skills::remove_skill_env_var),
        )
        // Channel
        .route("/channel/plugins", get(routes::channel::list_plugins))
        .route("/channel/accounts", get(routes::channel::list_accounts))
        .route("/channel/accounts", post(routes::channel::add_account))
        .route(
            "/channel/accounts/{id}",
            put(routes::channel::update_account),
        )
        .route(
            "/channel/accounts/{id}",
            delete(routes::channel::remove_account),
        )
        .route(
            "/channel/accounts/{id}/start",
            post(routes::channel::start_account),
        )
        .route(
            "/channel/accounts/{id}/stop",
            post(routes::channel::stop_account),
        )
        .route(
            "/channel/accounts/{id}/health",
            get(routes::channel::health),
        )
        .route(
            "/channel/accounts/{id}/test-message",
            post(routes::channel::send_test_message),
        )
        .route("/channel/health", get(routes::channel::health_all))
        .route(
            "/channel/validate",
            post(routes::channel::validate_credentials),
        )
        .route("/channel/sessions", get(routes::channel::list_sessions))
        .route(
            "/channel/wechat/login/start",
            post(routes::channel::wechat_start_login),
        )
        .route(
            "/channel/wechat/login/wait",
            post(routes::channel::wechat_wait_login),
        )
        // Crash / Backup
        .route(
            "/crash/recovery-info",
            get(routes::crash::get_crash_recovery_info),
        )
        .route("/crash/history", get(routes::crash::get_crash_history))
        .route("/crash/history", delete(routes::crash::clear_crash_history))
        .route("/crash/backups", get(routes::crash::list_backups))
        .route("/crash/backups", post(routes::crash::create_backup))
        .route(
            "/crash/backups/restore",
            post(routes::crash::restore_backup),
        )
        .route(
            "/settings/backups",
            get(routes::crash::list_settings_backups),
        )
        .route(
            "/settings/backups/restore",
            post(routes::crash::restore_settings_backup),
        )
        .route("/crash/guardian", get(routes::crash::get_guardian_enabled))
        .route("/crash/guardian", put(routes::crash::set_guardian_enabled))
        // URL Preview
        .route("/url-preview", post(routes::url_preview::fetch_url_preview))
        .route(
            "/url-preview/batch",
            post(routes::url_preview::fetch_url_previews),
        )
        // Embedded browser
        .route("/browser/status", get(routes::browser::get_status))
        .route(
            "/browser/profiles",
            get(routes::browser::list_profiles).post(routes::browser::create_profile),
        )
        .route(
            "/browser/profiles/{name}",
            delete(routes::browser::delete_profile),
        )
        .route("/browser/launch", post(routes::browser::launch))
        .route("/browser/connect", post(routes::browser::connect))
        .route("/browser/disconnect", post(routes::browser::disconnect))
        // Subagent
        .route("/subagent/runs", get(routes::subagent::list_subagent_runs))
        .route(
            "/subagent/runs/batch",
            post(routes::subagent::get_subagent_runs_batch),
        )
        .route(
            "/subagent/runs/{run_id}",
            get(routes::subagent::get_subagent_run),
        )
        .route(
            "/subagent/runs/{run_id}/kill",
            post(routes::subagent::kill_subagent),
        )
        // Agent Team
        .route("/teams", get(routes::team::list_teams).post(routes::team::create_team))
        .route("/teams/{id}", get(routes::team::get_team))
        .route("/teams/{id}/members", get(routes::team::get_team_members))
        .route("/teams/{id}/messages", get(routes::team::get_team_messages).post(routes::team::send_user_team_message))
        .route("/teams/{id}/tasks", get(routes::team::get_team_tasks))
        .route("/teams/{id}/pause", post(routes::team::pause_team))
        .route("/teams/{id}/resume", post(routes::team::resume_team))
        .route("/teams/{id}/dissolve", post(routes::team::dissolve_team))
        .route(
            "/team-templates",
            get(routes::team::list_team_templates).post(routes::team::save_team_template),
        )
        .route(
            "/team-templates/{id}",
            axum::routing::delete(routes::team::delete_team_template),
        )
        // ACP Control
        .route("/acp/backends", get(routes::acp::list_backends))
        .route("/acp/health-check", get(routes::acp::health_check))
        .route("/acp/refresh", post(routes::acp::refresh_backends))
        .route("/acp/runs", get(routes::acp::list_runs))
        .route("/acp/runs/{run_id}/kill", post(routes::acp::kill_run))
        .route(
            "/acp/runs/{run_id}/result",
            get(routes::acp::get_run_result),
        )
        .route("/acp/config", get(routes::acp::get_config))
        .route("/acp/config", put(routes::acp::set_config))
        // Weather
        .route("/weather/geocode", get(routes::weather::geocode_search))
        .route("/weather/preview", post(routes::weather::preview_weather))
        .route(
            "/weather/current",
            get(routes::weather::get_current_weather),
        )
        .route("/weather/refresh", post(routes::weather::refresh_weather))
        .route(
            "/weather/detect-location",
            get(routes::weather::detect_location),
        )
        // Slash commands
        .route("/slash-commands", get(routes::slash::list_slash_commands))
        .route(
            "/slash-commands/execute",
            post(routes::slash::execute_slash_command),
        )
        .route(
            "/slash-commands/is-slash",
            post(routes::slash::is_slash_command),
        )
        // Canvas
        .route(
            "/canvas/snapshot/{request_id}",
            post(routes::canvas::canvas_submit_snapshot),
        )
        .route(
            "/canvas/eval/{request_id}",
            post(routes::canvas::canvas_submit_eval_result),
        )
        .route("/canvas/show", post(routes::canvas::show_canvas_panel))
        // Providers extras
        .route(
            "/providers/available-models",
            get(routes::providers::get_available_models),
        )
        .route(
            "/providers/reorder",
            post(routes::providers::reorder_providers),
        )
        // Misc
        .route(
            "/misc/write-export-file",
            post(routes::misc::write_export_file),
        )
        // SearXNG Docker
        .route("/searxng/status", get(routes::searxng::status))
        .route("/searxng/deploy", post(routes::searxng::deploy))
        .route("/searxng/start", post(routes::searxng::start))
        .route("/searxng/stop", post(routes::searxng::stop))
        .route("/searxng", delete(routes::searxng::remove))
        // Auth
        .route(
            "/auth/codex/start",
            post(routes::auth::start_codex_auth),
        )
        .route(
            "/auth/codex/finalize",
            post(routes::auth::finalize_codex_auth),
        )
        // System (desktop-only stubs)
        .route(
            "/system/restart",
            post(routes::system::request_app_restart),
        )
        // Desktop (desktop-only stubs)
        .route("/desktop/open-url", post(routes::desktop::open_url))
        .route(
            "/desktop/open-directory",
            post(routes::desktop::open_directory),
        )
        .route(
            "/desktop/reveal-in-folder",
            post(routes::desktop::reveal_in_folder),
        )
        // Dev tools
        .route("/dev/clear-sessions", post(routes::dev::clear_sessions))
        .route("/dev/clear-cron", post(routes::dev::clear_cron))
        .route("/dev/clear-memory", post(routes::dev::clear_memory))
        .route("/dev/reset-config", post(routes::dev::reset_config))
        .route("/dev/clear-all", post(routes::dev::clear_all));

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
        .layer(axum::middleware::from_fn(middleware::access_log))
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
        let parsed: Vec<_> = origins.iter().filter_map(|o| o.parse().ok()).collect();
        cors.allow_origin(parsed)
    }
}
