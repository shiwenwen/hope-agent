// Hope Agent HTTP/WebSocket Server
// Depends on ha-core for business logic, uses axum 0.8 for HTTP.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, patch, post, put};
use axum::Router;
use tower_http::cors::{AllowOrigin, CorsLayer};

use ha_core::event_bus::EventBus;
use ha_core::project::ProjectDB;
use ha_core::session::SessionDB;

pub mod auto_approve;
pub mod banner;
pub mod config;
pub mod error;
pub mod middleware;
pub mod routes;
pub mod web_assets;
pub mod ws;

pub use config::ServerConfig;

// ── AppContext ───────────────────────────────────────────────────

/// Shared application state passed to all handlers via `State<Arc<AppContext>>`.
pub struct AppContext {
    pub session_db: Arc<SessionDB>,
    pub project_db: Arc<ProjectDB>,
    pub event_bus: Arc<dyn EventBus>,
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
    build_router_with_cors(ctx, &[], None, None)
}

/// Start the HTTP/WebSocket server, binding to the configured address.
///
/// Prints the structured `[ha-server] listening on ...` log line for log
/// aggregators as well as the human-readable launch banner (Web GUI URL,
/// API endpoint, API key). Both go to stderr so they don't contaminate
/// the ACP NDJSON stdout when the embedded server runs under
/// `hope-agent acp`.
pub async fn start_server(config: ServerConfig, ctx: Arc<AppContext>) -> anyhow::Result<()> {
    let router = build_router_with_cors(
        ctx,
        &config.cors_origins,
        config.api_key.clone(),
        config.knowledge_agent_read_token.clone(),
    );

    let listener = match tokio::net::TcpListener::bind(&config.bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            ha_core::server_status::mark_failed(format!("bind {}: {}", config.bind_addr, e));
            return Err(e.into());
        }
    };
    let actual_addr = listener.local_addr().unwrap_or_else(|_| {
        // Fallback to a parsed form of the configured string if the kernel
        // refuses local_addr() — we still want to mark "started" so the GUI
        // stops showing "Not started".
        config
            .bind_addr
            .parse()
            .unwrap_or_else(|_| "0.0.0.0:0".parse().expect("literal parses"))
    });
    ha_core::server_status::mark_started(actual_addr);

    eprintln!("[ha-server] listening on {}", actual_addr);
    banner::print_launch_banner(&actual_addr.to_string(), config.api_key.as_deref());

    // SessionEnd(shutdown) is fired from `crash_flush::run_clean_shutdown` — the
    // single signal-path chokepoint that actually runs on SIGTERM/SIGINT (it
    // `process::exit`s, so a graceful-shutdown future here would never win the
    // race). Plain serve; the signal handler terminates this future.
    if let Err(e) = axum::serve(listener, router).await {
        ha_core::server_status::mark_failed(format!("serve: {}", e));
        return Err(e.into());
    }
    Ok(())
}

// ── Internal Helpers ────────────────────────────────────────────

/// Build the router with specific CORS origins and optional API key auth.
fn build_router_with_cors(
    ctx: Arc<AppContext>,
    cors_origins: &[String],
    api_key: Option<String>,
    knowledge_agent_read_token: Option<String>,
) -> Router {
    // Health + server status are always public (no auth required). The
    // status payload only contains bound-addr / uptime / WS counts — nothing
    // secret — and keeping it unauthenticated lets the Transport layer probe
    // remote servers the same way it probes `/api/health`.
    let health = Router::new()
        .route("/api/health", get(routes::health::health_check))
        .route(
            "/api/server/status",
            get(routes::server_status::server_status),
        )
        // B7-1 只读分享：**公开无鉴权**——token 是唯一不可猜凭证，服务干净快照（sandbox
        // opaque-origin）。放公开路由（与 /api/health 同层），不进 require_api_key 保护面。
        .route(
            "/api/design/share/{token}",
            get(routes::design::serve_share),
        );

    // Protected API routes
    let api = Router::new()
        // Sessions
        .route("/sessions", post(routes::sessions::create_session))
        .route("/sessions", get(routes::sessions::list_sessions))
        .route("/sessions/{id}", get(routes::sessions::get_session))
        .route("/sessions/{id}", delete(routes::sessions::delete_session))
        .route("/sessions/{id}", patch(routes::sessions::rename_session))
        .route(
            "/sessions/{id}/pinned",
            patch(routes::sessions::set_session_pinned),
        )
        .route(
            "/sessions/{id}/incognito",
            patch(routes::sessions::set_session_incognito),
        )
        .route(
            "/sessions/{id}/working-dir",
            patch(routes::sessions::set_session_working_dir),
        )
        .route(
            "/sessions/{id}/agent",
            patch(routes::sessions::update_session_agent),
        )
        .route(
            "/sessions/{id}/model",
            patch(routes::sessions::set_session_model),
        )
        .route(
            "/sessions/{id}/purge-if-incognito",
            post(routes::sessions::purge_session_if_incognito),
        )
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
        .route(
            "/sessions/{id}/export",
            get(routes::sessions::export_session_http),
        )
        .route(
            "/sessions/{id}/files/by-path",
            get(routes::sessions::download_session_file_by_path),
        )
        .route(
            "/sessions/{id}/files/read",
            get(routes::sessions::read_session_file_by_path),
        )
        .route(
            "/sessions/{id}/files/extract",
            get(routes::sessions::extract_session_file_by_path),
        )
        .route(
            "/sessions/{id}/git-diff",
            get(routes::sessions::get_session_git_diff),
        )
        .route("/sessions/search", get(routes::sessions::search_sessions))
        // Projects
        .route("/projects", get(routes::projects::list_projects))
        .route("/projects", post(routes::projects::create_project))
        .route(
            "/projects/reorder",
            post(routes::projects::reorder_projects),
        )
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
            "/projects/{id}/read",
            post(routes::projects::mark_project_sessions_read),
        )
        .route(
            "/projects/{id}/memories",
            get(routes::projects::list_project_memories),
        )
        // ── Knowledge Base ──
        .route(
            "/knowledge",
            get(routes::knowledge::list_kbs).post(routes::knowledge::create_kb),
        )
        .route("/knowledge/search", get(routes::knowledge::kb_search))
        .route("/knowledge/attach", post(routes::knowledge::attach_kb))
        .route("/knowledge/detach", post(routes::knowledge::detach_kb))
        .route(
            "/knowledge/attachments",
            get(routes::knowledge::list_session_kbs),
        )
        .route(
            "/knowledge/project-attachments",
            get(routes::knowledge::list_project_kbs),
        )
        .route(
            "/knowledge/{kb_id}",
            get(routes::knowledge::get_kb)
                .patch(routes::knowledge::update_kb)
                .delete(routes::knowledge::delete_kb),
        )
        .route(
            "/knowledge/{kb_id}/reindex",
            post(routes::knowledge::reindex_kb),
        )
        .route(
            "/knowledge/{kb_id}/notes",
            get(routes::knowledge::list_kb_notes),
        )
        .route(
            "/knowledge/{kb_id}/note",
            get(routes::knowledge::kb_note_read)
                .put(routes::knowledge::kb_note_save)
                .delete(routes::knowledge::kb_note_delete),
        )
        .route(
            "/knowledge/{kb_id}/note/rename",
            post(routes::knowledge::kb_note_rename),
        )
        .route(
            "/knowledge/{kb_id}/note/reindex",
            post(routes::knowledge::reindex_note),
        )
        .route(
            "/knowledge/{kb_id}/dir/reindex",
            post(routes::knowledge::reindex_dir),
        )
        .route(
            "/knowledge/{kb_id}/sources",
            get(routes::knowledge::kb_source_list)
                .post(routes::knowledge::kb_source_import)
                .layer(DefaultBodyLimit::max(
                    (ha_core::knowledge::source::MAX_BINARY_SOURCE_BYTES * 4 / 3) + 2 * 1024 * 1024,
                )),
        )
        .route(
            "/knowledge/{kb_id}/sources/browser",
            post(routes::knowledge::kb_source_import_browser),
        )
        .route(
            "/knowledge/{kb_id}/sources/session-attachment",
            post(routes::knowledge::kb_source_import_session_attachment),
        )
        .route(
            "/knowledge/{kb_id}/sources/batch",
            post(routes::knowledge::kb_source_import_batch).layer(DefaultBodyLimit::max(
                (ha_core::knowledge::source::MAX_BINARY_SOURCE_BYTES * 4 / 3 * 3) + 4 * 1024 * 1024,
            )),
        )
        .route(
            "/knowledge/{kb_id}/sources/import-runs",
            get(routes::knowledge::kb_source_import_runs_list),
        )
        .route(
            "/knowledge/{kb_id}/sources/import-runs/{run_id}",
            get(routes::knowledge::kb_source_import_run_detail),
        )
        .route(
            "/knowledge/{kb_id}/sources/import-runs/{run_id}/retry-failed",
            post(routes::knowledge::kb_source_import_retry_failed),
        )
        .route(
            "/knowledge/{kb_id}/sources/similar",
            get(routes::knowledge::kb_source_similarity_groups),
        )
        .route(
            "/knowledge/{kb_id}/sources/similar/dismiss",
            post(routes::knowledge::kb_source_similarity_dismiss),
        )
        .route(
            "/knowledge/{kb_id}/sources/similar/resolve",
            post(routes::knowledge::kb_source_similarity_resolve),
        )
        .route(
            "/knowledge/{kb_id}/sources/sync-external-raw",
            post(routes::knowledge::kb_source_sync_external_raw),
        )
        .route(
            "/knowledge/{kb_id}/sources/{source_id}/assets/{asset_kind}/link",
            get(routes::knowledge::kb_source_asset_link),
        )
        .route(
            "/knowledge/{kb_id}/sources/{source_id}/assets/{asset_kind}",
            get(routes::knowledge::kb_source_asset_file),
        )
        .route(
            "/knowledge/{kb_id}/sources/{source_id}",
            get(routes::knowledge::kb_source_read).delete(routes::knowledge::kb_source_delete),
        )
        .route(
            "/knowledge/{kb_id}/sources/{source_id}/refresh",
            post(routes::knowledge::kb_source_refresh),
        )
        .route(
            "/knowledge/{kb_id}/sources/{source_id}/versions",
            get(routes::knowledge::kb_source_versions),
        )
        .route(
            "/knowledge/{kb_id}/sources/{source_id}/diff",
            get(routes::knowledge::kb_source_diff),
        )
        .route(
            "/knowledge/{kb_id}/sources/{source_id}/reextract",
            post(routes::knowledge::kb_source_reextract),
        )
        .route(
            "/knowledge/{kb_id}/compile-runs",
            get(routes::knowledge::kb_compile_runs_list).post(routes::knowledge::kb_compile_start),
        )
        .route(
            "/knowledge/{kb_id}/compile-runs/{run_id}",
            get(routes::knowledge::kb_compile_status),
        )
        .route(
            "/knowledge/{kb_id}/compile-runs/{run_id}/cancel",
            post(routes::knowledge::kb_compile_run_cancel),
        )
        .route(
            "/knowledge/{kb_id}/compile-proposals",
            get(routes::knowledge::kb_compile_proposals_list),
        )
        .route(
            "/knowledge/{kb_id}/compile-proposals/{id}/approve",
            post(routes::knowledge::kb_compile_proposal_approve),
        )
        .route(
            "/knowledge/{kb_id}/compile-proposals/{id}/reject",
            post(routes::knowledge::kb_compile_proposal_reject),
        )
        .route(
            "/knowledge/{kb_id}/query-file",
            post(routes::knowledge::kb_query_file),
        )
        .route(
            "/knowledge/{kb_id}/schema-profile",
            get(routes::knowledge::kb_schema_profile),
        )
        .route(
            "/knowledge/{kb_id}/schema-issues",
            get(routes::knowledge::kb_schema_issues),
        )
        .route(
            "/knowledge/{kb_id}/note/source-refs",
            get(routes::knowledge::kb_note_source_refs),
        )
        .route(
            "/knowledge/{kb_id}/evidence/coverage",
            get(routes::knowledge::kb_evidence_coverage),
        )
        .route(
            "/knowledge/{kb_id}/evidence/sources/{source_id}/claims",
            get(routes::knowledge::kb_evidence_source_claims),
        )
        .route(
            "/knowledge/{kb_id}/evidence/rebuild",
            post(routes::knowledge::kb_evidence_rebuild),
        )
        .route(
            "/knowledge/agent/search",
            post(routes::knowledge::knowledge_agent_search),
        )
        .route(
            "/knowledge/agent/read",
            post(routes::knowledge::knowledge_agent_read),
        )
        .route(
            "/knowledge/agent/expand",
            post(routes::knowledge::knowledge_agent_expand),
        )
        .route(
            "/knowledge/agent/sources",
            post(routes::knowledge::knowledge_agent_sources),
        )
        .route(
            "/knowledge/agent/compile/propose",
            post(routes::knowledge::knowledge_agent_compile_propose),
        )
        .route(
            "/knowledge/{kb_id}/dirs",
            get(routes::knowledge::kb_list_dirs),
        )
        .route(
            "/knowledge/{kb_id}/tags",
            get(routes::knowledge::kb_list_tags),
        )
        .route(
            "/knowledge/referenceable-notes",
            post(routes::knowledge::list_referenceable_notes),
        )
        .route(
            "/knowledge/embedding",
            get(routes::knowledge::knowledge_embedding_get),
        )
        .route(
            "/knowledge/embedding/set-default",
            post(routes::knowledge::knowledge_embedding_set_default),
        )
        .route(
            "/knowledge/embedding/disable",
            post(routes::knowledge::knowledge_embedding_disable),
        )
        .route(
            "/knowledge/embedding/rebuild",
            post(routes::knowledge::knowledge_embedding_rebuild),
        )
        .route(
            "/knowledge/chunk",
            get(routes::knowledge::knowledge_chunk_get)
                .post(routes::knowledge::knowledge_chunk_set),
        )
        .route(
            "/knowledge/search-config",
            get(routes::knowledge::knowledge_search_config_get)
                .post(routes::knowledge::knowledge_search_config_set),
        )
        .route(
            "/knowledge/compile/config",
            get(routes::knowledge::knowledge_compile_config_get)
                .post(routes::knowledge::knowledge_compile_config_set),
        )
        .route(
            "/knowledge/ai/rewrite",
            post(routes::knowledge::kb_ai_rewrite),
        )
        .route(
            "/knowledge/rewrite/log",
            post(routes::knowledge::kb_rewrite_log),
        )
        .route(
            "/knowledge/maintenance/run",
            post(routes::knowledge::kb_maintenance_run),
        )
        .route(
            "/knowledge/maintenance/status",
            get(routes::knowledge::kb_maintenance_status),
        )
        .route(
            "/knowledge/maintenance/proposals/{id}/approve",
            post(routes::knowledge::kb_maintenance_approve),
        )
        .route(
            "/knowledge/maintenance/proposals/{id}/reject",
            post(routes::knowledge::kb_maintenance_reject),
        )
        .route(
            "/knowledge/{kb_id}/maintenance/proposals",
            get(routes::knowledge::kb_maintenance_list),
        )
        .route(
            "/knowledge/{kb_id}/maintenance/pending-count",
            get(routes::knowledge::kb_maintenance_pending_count),
        )
        .route(
            "/knowledge/{kb_id}/maintenance/reject-all",
            post(routes::knowledge::kb_maintenance_reject_all),
        )
        .route(
            "/knowledge/maintenance/config",
            get(routes::knowledge::kb_maintenance_config_get)
                .post(routes::knowledge::kb_maintenance_config_set),
        )
        .route(
            "/knowledge/passive-recall/config",
            get(routes::knowledge::kb_passive_recall_config_get)
                .post(routes::knowledge::kb_passive_recall_config_set),
        )
        .route(
            "/knowledge/media-retention/config",
            get(routes::knowledge::knowledge_media_retention_config_get)
                .post(routes::knowledge::knowledge_media_retention_config_set),
        )
        .route(
            "/knowledge/sprite/observe",
            post(routes::knowledge::kb_sprite_observe),
        )
        .route(
            "/knowledge/sprite/config",
            get(routes::knowledge::sprite_config_get).post(routes::knowledge::sprite_config_set),
        )
        .route(
            "/knowledge/{kb_id}/dir",
            post(routes::knowledge::kb_mkdir).delete(routes::knowledge::kb_delete_dir),
        )
        .route(
            "/knowledge/{kb_id}/dir/rename",
            post(routes::knowledge::kb_rename_dir),
        )
        .route(
            "/knowledge/{kb_id}/backlinks",
            get(routes::knowledge::kb_backlinks),
        )
        .route(
            "/knowledge/{kb_id}/broken-links",
            get(routes::knowledge::kb_broken_links),
        )
        .route(
            "/knowledge/{kb_id}/orphans",
            get(routes::knowledge::kb_orphans),
        )
        .route("/knowledge/{kb_id}/graph", get(routes::knowledge::kb_graph))
        .route(
            "/knowledge/{kb_id}/graph/layout",
            get(routes::knowledge::kb_graph_layout_get)
                .post(routes::knowledge::kb_graph_layout_save),
        )
        .route(
            "/knowledge/{kb_id}/chat/thread",
            get(routes::knowledge::kb_chat_thread_get),
        )
        .route(
            "/knowledge/{kb_id}/chat/threads",
            get(routes::knowledge::kb_chat_threads_list),
        )
        .route(
            "/knowledge/{kb_id}/note/resolve",
            get(routes::knowledge::kb_note_read_ref),
        )
        .route(
            "/knowledge/{kb_id}/files/read",
            get(routes::knowledge::kb_file_read),
        )
        .route(
            "/knowledge/{kb_id}/files/extract",
            get(routes::knowledge::kb_file_extract),
        )
        .route(
            "/knowledge/{kb_id}/files/raw",
            get(routes::knowledge::kb_file_raw),
        )
        .route(
            "/sessions/{id}/messages/around",
            get(routes::sessions::get_session_messages_around),
        )
        .route(
            "/sessions/{id}/messages/before",
            get(routes::sessions::get_session_messages_before),
        )
        .route(
            "/sessions/{id}/messages/after",
            get(routes::sessions::get_session_messages_after),
        )
        .route(
            "/sessions/{id}/messages/search",
            get(routes::sessions::search_session_messages),
        )
        .route(
            "/sessions/{id}/artifacts",
            get(routes::sessions::get_session_artifacts),
        )
        .route(
            "/sessions/{id}/background-jobs",
            get(routes::sessions::list_session_background_jobs),
        )
        .route(
            "/background-jobs/{job_id}",
            get(routes::sessions::get_background_job),
        )
        .route(
            "/sessions/{id}/environment",
            get(routes::sessions::get_session_environment),
        )
        .route(
            "/sessions/{id}/stream-state",
            get(routes::sessions::get_session_stream_state),
        )
        // Chat
        .route("/chat", post(routes::chat::chat))
        .route(
            "/chat/turn-message",
            post(routes::chat::queue_turn_user_message),
        )
        .route(
            "/chat/turn-message/cancel",
            post(routes::chat::cancel_queued_turn_user_message),
        )
        .route("/chat/stop", post(routes::chat::stop_chat))
        .route(
            "/sessions/{sessionId}/tasks",
            get(routes::tasks::list_session_tasks),
        )
        .route(
            "/tasks/{id}/status",
            patch(routes::tasks::update_task_status),
        )
        .route("/tasks/{id}", delete(routes::tasks::delete_task))
        .route(
            "/runtime-tasks/cancel",
            post(routes::runtime_tasks::cancel_runtime_task),
        )
        .route(
            "/chat/permission-mode",
            post(routes::chat::set_permission_mode),
        )
        .route("/chat/sandbox-mode", post(routes::chat::set_sandbox_mode))
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
            post(routes::chat::save_attachment).layer(DefaultBodyLimit::max(25 * 1024 * 1024)),
        )
        // Attachment download (serves session-scoped files under
        // ~/.hope-agent/attachments/{session_id}/) — the logical URL
        // form emitted in `__MEDIA_ITEMS__` events.
        .route(
            "/attachments/{session_id}/{filename}",
            get(routes::attachments::download),
        )
        // Avatar download + upload — serves and persists files under
        // `~/.hope-agent/avatars/{filename}`. Used by agent / user-profile
        // avatar UI in HTTP mode where Tauri's `convertFileSrc` /
        // `@tauri-apps/plugin-dialog` aren't available.
        .route("/avatars/{filename}", get(routes::avatars::download))
        .route(
            "/avatars",
            post(routes::avatars::upload).layer(DefaultBodyLimit::max(10 * 1024 * 1024)),
        )
        // Generated-image download — serves historic `~/.hope-agent/image_generate/*.png`
        // referenced by legacy `mediaUrls` absolute-path rows. Used by
        // `ToolMediaPreview` in HTTP mode.
        .route(
            "/generated-images/{filename}",
            get(routes::generated_images::download),
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
        .route("/providers/test-model", post(routes::providers::test_model))
        .route("/providers/has-any", get(routes::providers::has_providers))
        .route(
            "/providers/active-model",
            get(routes::providers::get_active_model),
        )
        .route(
            "/providers/active-model",
            put(routes::providers::set_active_model),
        )
        // MCP (Model Context Protocol) servers
        .route("/mcp/servers", get(routes::mcp::list_servers))
        .route("/mcp/servers", post(routes::mcp::add_server))
        .route("/mcp/servers/reorder", post(routes::mcp::reorder_servers))
        .route("/mcp/servers/{id}", put(routes::mcp::update_server))
        .route("/mcp/servers/{id}", delete(routes::mcp::remove_server))
        .route(
            "/mcp/servers/{id}/status",
            get(routes::mcp::get_server_status),
        )
        .route("/mcp/servers/{id}/test", post(routes::mcp::test_connection))
        .route(
            "/mcp/servers/{id}/reconnect",
            post(routes::mcp::reconnect_server),
        )
        .route(
            "/mcp/servers/{id}/oauth/start",
            post(routes::mcp::start_oauth),
        )
        .route(
            "/mcp/servers/{id}/oauth/sign-out",
            post(routes::mcp::sign_out),
        )
        .route("/mcp/servers/{id}/tools", get(routes::mcp::list_tools))
        .route("/mcp/servers/{id}/logs", get(routes::mcp::get_recent_logs))
        .route(
            "/mcp/import/claude-desktop",
            post(routes::mcp::import_claude_desktop_config),
        )
        .route("/mcp/global", get(routes::mcp::get_global_settings))
        .route("/mcp/global", put(routes::mcp::update_global_settings))
        // Models (aliases under /api/models/*)
        .route("/models", get(routes::models::list_available_models))
        .route("/models/active", get(routes::models::get_active_model))
        .route("/models/active", post(routes::models::set_active_model))
        .route("/models/fallback", get(routes::models::get_fallback_models))
        .route(
            "/models/fallback",
            post(routes::models::set_fallback_models),
        )
        .route(
            "/models/reasoning-effort",
            post(routes::models::set_reasoning_effort),
        )
        .route(
            "/models/settings",
            get(routes::models::get_current_settings),
        )
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
        .route("/claims", get(routes::memory::list_claims))
        .route("/claims/{id}", get(routes::memory::get_claim))
        .route("/claims/{id}", patch(routes::memory::update_claim))
        .route("/claims/{id}/forget", post(routes::memory::forget_claim))
        .route("/memory/{id}", get(routes::memory::get_memory))
        .route("/memory/{id}", put(routes::memory::update_memory))
        .route("/memory/{id}", delete(routes::memory::delete_memory))
        .route("/memory/search", post(routes::memory::search_memories))
        .route(
            "/memory/backfill/plan",
            get(routes::memory::memory_backfill_plan),
        )
        .route(
            "/memory/backfill/apply",
            post(routes::memory::memory_backfill_apply),
        )
        .route("/memory/count", get(routes::memory::memory_count))
        .route("/memory/stats", get(routes::memory::memory_stats))
        .route(
            "/memory/import-from-ai-prompt",
            get(routes::memory::import_from_ai_prompt),
        )
        .route("/memory/{id}/pin", post(routes::memory::toggle_pin))
        .route("/memory/delete-batch", post(routes::memory::delete_batch))
        .route("/memory/reembed", post(routes::memory::reembed))
        .route("/memory/reembed-start", post(routes::memory::reembed_start))
        .route("/memory/export", post(routes::memory::export_memory))
        .route(
            "/memory/import",
            post(routes::memory::import_memory).layer(DefaultBodyLimit::max(25 * 1024 * 1024)),
        )
        .route("/memory/find-similar", post(routes::memory::find_similar))
        .route(
            "/memory/global-md",
            get(routes::memory::get_global_memory_md),
        )
        .route(
            "/memory/global-md",
            put(routes::memory::save_global_memory_md),
        )
        .route(
            "/memory/local-embedding-models",
            get(routes::memory::list_local_embedding_models),
        )
        // Local Ollama embedding assistant
        .route(
            "/local-embedding/models",
            get(routes::local_embedding::list_models),
        )
        // Local model background jobs
        .route(
            "/local-model-jobs",
            get(routes::local_model_jobs::list_jobs),
        )
        .route(
            "/local-model-jobs/chat-model",
            post(routes::local_model_jobs::start_chat_model),
        )
        .route(
            "/local-model-jobs/embedding",
            post(routes::local_model_jobs::start_embedding),
        )
        .route(
            "/local-model-jobs/ollama-install",
            post(routes::local_model_jobs::start_ollama_install),
        )
        .route(
            "/local-model-jobs/ollama-pull",
            post(routes::local_model_jobs::start_ollama_pull),
        )
        .route(
            "/local-model-jobs/ollama-preload",
            post(routes::local_model_jobs::start_ollama_preload),
        )
        .route(
            "/local-model-jobs/{id}",
            get(routes::local_model_jobs::get_job).delete(routes::local_model_jobs::clear_job),
        )
        .route(
            "/local-model-jobs/{id}/logs",
            get(routes::local_model_jobs::get_logs),
        )
        .route(
            "/local-model-jobs/{id}/cancel",
            post(routes::local_model_jobs::cancel_job),
        )
        .route(
            "/local-model-jobs/{id}/pause",
            post(routes::local_model_jobs::pause_job),
        )
        .route(
            "/local-model-jobs/{id}/retry",
            post(routes::local_model_jobs::retry_job),
        )
        .route(
            "/local-model/alert/dismiss-temporary",
            post(routes::local_model_alerts::dismiss_temporary),
        )
        .route(
            "/local-model/alert/silence-session",
            post(routes::local_model_alerts::silence_session),
        )
        .route(
            "/local-model/auto-maintenance",
            get(routes::local_model_alerts::get_auto_maintenance)
                .put(routes::local_model_alerts::set_auto_maintenance),
        )
        .route(
            "/local-model/auto-maintenance/disable",
            post(routes::local_model_alerts::disable),
        )
        .route(
            "/local-model/auto-maintenance/trigger",
            post(routes::local_model_alerts::trigger),
        )
        // Config
        .route("/config/user", get(routes::config::get_user_config))
        .route("/config/user", put(routes::config::save_user_config))
        .route(
            "/config/default-agent",
            get(routes::config::get_default_agent_id),
        )
        .route(
            "/config/default-agent",
            put(routes::config::set_default_agent_id),
        )
        .route(
            "/config/web-search",
            get(routes::config::get_web_search_config),
        )
        .route(
            "/config/web-search",
            put(routes::config::save_web_search_config),
        )
        .route(
            "/config/issue-reporting",
            get(routes::config::get_issue_reporting_config),
        )
        .route(
            "/config/issue-reporting",
            put(routes::config::save_issue_reporting_config),
        )
        .route(
            "/config/issue-reporting/token",
            put(routes::config::save_issue_reporting_token),
        )
        .route(
            "/config/issue-reporting/test",
            post(routes::config::test_issue_reporting_connection),
        )
        .route("/config/proxy", get(routes::config::get_proxy_config))
        .route("/config/proxy", put(routes::config::save_proxy_config))
        .route(
            "/config/proxy/test",
            post(routes::config::test_proxy_config),
        )
        .route("/config/compact", get(routes::config::get_compact_config))
        .route("/config/compact", put(routes::config::save_compact_config))
        .route("/config/hooks", get(routes::config::get_hooks_config))
        .route("/config/hooks", put(routes::config::save_hooks_config))
        .route(
            "/config/session-title",
            get(routes::config::get_session_title_config),
        )
        .route(
            "/config/session-title",
            put(routes::config::save_session_title_config),
        )
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
        .route("/config/dreaming", get(routes::config::get_dreaming_config))
        .route(
            "/config/dreaming",
            put(routes::config::save_dreaming_config),
        )
        .route(
            "/config/async-tools",
            get(routes::config::get_async_tools_config),
        )
        .route(
            "/config/async-tools",
            put(routes::config::save_async_tools_config),
        )
        .route("/config/cron", get(routes::config::get_cron_config))
        .route("/config/cron", put(routes::config::save_cron_config))
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
            "/config/auto-update",
            get(routes::config::get_auto_update_config),
        )
        .route(
            "/config/auto-update",
            put(routes::config::set_auto_update_config),
        )
        .route(
            "/config/startup-notification",
            get(routes::config::get_startup_notification_config),
        )
        .route(
            "/config/startup-notification",
            put(routes::config::save_startup_notification_config),
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
            "/config/timeout-policy",
            get(routes::config::get_timeout_policy_config),
        )
        .route(
            "/config/timeout-policy",
            put(routes::config::save_timeout_policy_config),
        )
        .route(
            "/config/approval-timeout",
            get(routes::config::get_approval_timeout),
        )
        .route(
            "/config/approval-timeout-enabled",
            get(routes::config::get_approval_timeout_enabled),
        )
        .route(
            "/config/approval-timeout",
            post(routes::config::set_approval_timeout),
        )
        .route(
            "/config/approval-timeout-enabled",
            post(routes::config::set_approval_timeout_enabled),
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
            "/config/unattended-approval-action",
            get(routes::config::get_unattended_approval_action),
        )
        .route(
            "/config/unattended-approval-action",
            post(routes::config::set_unattended_approval_action),
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
            "/config/ask-user-question-timeout-enabled",
            get(routes::config::get_ask_user_question_timeout_enabled),
        )
        .route(
            "/config/ask-user-question-timeout",
            post(routes::config::set_ask_user_question_timeout),
        )
        .route(
            "/config/ask-user-question-timeout-enabled",
            post(routes::config::set_ask_user_question_timeout_enabled),
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
            "/config/embedding-models",
            get(routes::config::embedding_model_config_list)
                .put(routes::config::embedding_model_config_save),
        )
        .route(
            "/config/embedding-models/templates",
            get(routes::config::embedding_model_config_templates),
        )
        .route(
            "/config/embedding-models/delete",
            post(routes::config::embedding_model_config_delete),
        )
        .route(
            "/config/embedding-models/test",
            post(routes::config::embedding_model_config_test),
        )
        .route(
            "/config/memory-embedding",
            get(routes::config::memory_embedding_get),
        )
        .route(
            "/config/memory-embedding/default",
            post(routes::config::memory_embedding_set_default),
        )
        .route(
            "/config/memory-embedding/disable",
            post(routes::config::memory_embedding_disable),
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
            "/config/filesystem",
            get(routes::config::get_filesystem_config).put(routes::config::save_filesystem_config),
        )
        .route(
            "/config/image-generate",
            get(routes::config::get_image_generate_config),
        )
        .route(
            "/config/image-generate",
            put(routes::config::save_image_generate_config),
        )
        .route(
            "/config/audio-generate",
            get(routes::config::get_audio_generate_config)
                .put(routes::config::save_audio_generate_config),
        )
        .route("/config/canvas", get(routes::config::get_canvas_config))
        .route("/config/canvas", put(routes::config::save_canvas_config))
        .route("/config/design", get(routes::config::get_design_config))
        .route("/config/design", put(routes::config::save_design_config))
        .route("/config/sandbox", get(routes::config::get_sandbox_config))
        .route("/config/sandbox", put(routes::config::set_sandbox_config))
        .route(
            "/config/sandbox/status",
            get(routes::config::get_sandbox_status),
        )
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
        // Config — quick prompts
        .route(
            "/config/quick-prompts",
            get(routes::config::get_quick_prompt_config).post(routes::config::add_quick_prompt),
        )
        // Config — theme / language / UI
        .route("/config/theme", get(routes::config::get_theme))
        .route("/config/theme", post(routes::config::set_theme))
        .route(
            "/config/window-theme",
            post(routes::config::set_window_theme),
        )
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
            "/config/prevent-sleep",
            get(routes::config::get_prevent_sleep_enabled),
        )
        .route(
            "/config/prevent-sleep",
            post(routes::config::set_prevent_sleep_enabled),
        )
        .route(
            "/config/sidebar-display-mode",
            get(routes::config::get_sidebar_display_mode),
        )
        .route(
            "/config/sidebar-display-mode",
            post(routes::config::set_sidebar_display_mode),
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
        .route("/agents/reorder", post(routes::agents::reorder_agents))
        .route("/agents/template", get(routes::agents::get_agent_template))
        .route("/agents/initialize", post(routes::agents::initialize_agent))
        .route(
            "/agents/openclaw/scan",
            get(routes::agents::scan_openclaw_agents),
        )
        .route(
            "/agents/openclaw/import",
            post(routes::agents::import_openclaw_agents),
        )
        .route(
            "/agents/openclaw/scan-full",
            get(routes::agents::scan_openclaw_full),
        )
        .route(
            "/agents/openclaw/import-full",
            post(routes::agents::import_openclaw_full),
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
        .route(
            "/cron/jobs-referencing-account/{account_id}",
            get(routes::cron::jobs_referencing_account),
        )
        .route("/cron/calendar", get(routes::cron::get_calendar_events))
        .route("/cron/timeline", get(routes::cron::run_timeline))
        .route("/cron/unread", get(routes::cron::unread_total))
        .route("/cron/read-all", post(routes::cron::mark_all_read))
        // Dreaming (offline memory consolidation, Phase B3)
        .route("/dreaming/run", post(routes::dreaming::run_now))
        .route("/dreaming/resolver", post(routes::dreaming::run_resolver))
        .route("/dreaming/profile/run", post(routes::dreaming::run_profile))
        .route(
            "/dreaming/profile",
            get(routes::dreaming::list_profile_snapshots),
        )
        .route("/dreaming/diaries", get(routes::dreaming::list_diaries))
        .route(
            "/dreaming/diaries/{filename}",
            get(routes::dreaming::read_diary),
        )
        .route("/dreaming/status", get(routes::dreaming::status))
        .route("/dreaming/last-report", get(routes::dreaming::last_report))
        .route("/dreaming/idle-status", get(routes::dreaming::idle_status))
        .route("/dreaming/runs", get(routes::dreaming::list_runs))
        .route("/dreaming/runs/{id}", get(routes::dreaming::get_run))
        .route(
            "/dreaming/evidence/quote",
            get(routes::dreaming::evidence_quote),
        )
        .route(
            "/cron/validate",
            post(routes::config::validate_cron_expression),
        )
        // Onboarding wizard
        .route("/onboarding/state", get(routes::onboarding::get_state))
        .route("/onboarding/draft", post(routes::onboarding::save_draft))
        .route(
            "/onboarding/complete",
            post(routes::onboarding::mark_completed),
        )
        .route("/onboarding/skip", post(routes::onboarding::mark_skipped))
        .route("/onboarding/reset", post(routes::onboarding::reset))
        .route(
            "/onboarding/language",
            post(routes::onboarding::apply_language),
        )
        .route(
            "/onboarding/profile",
            post(routes::onboarding::apply_profile),
        )
        .route(
            "/onboarding/personality-preset",
            post(routes::onboarding::apply_personality_preset),
        )
        .route("/onboarding/safety", post(routes::onboarding::apply_safety))
        .route("/onboarding/skills", post(routes::onboarding::apply_skills))
        .route("/onboarding/server", post(routes::onboarding::apply_server))
        .route(
            "/server/generate-api-key",
            post(routes::onboarding::generate_api_key),
        )
        .route("/server/local-ips", get(routes::onboarding::list_local_ips))
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
        .route("/dashboard/plan-stats", post(routes::dashboard::plan_stats))
        .route(
            "/dashboard/local-model-usage",
            post(routes::dashboard::local_model_usage),
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
        .route("/plan/list", post(routes::plan::list_plans))
        .route(
            "/plan/resolve-mention",
            post(routes::plan::resolve_plan_mention),
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
            "/skills/mentionable",
            get(routes::skills::list_mentionable_skills),
        )
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
            "/skills/auto-review/promotion",
            get(routes::skills::get_auto_review_promotion)
                .put(routes::skills::set_auto_review_promotion),
        )
        .route(
            "/skills/auto-review/enabled",
            get(routes::skills::get_auto_review_enabled)
                .put(routes::skills::set_auto_review_enabled),
        )
        .route(
            "/skills/auto-review/config",
            get(routes::skills::get_auto_review_config)
                .patch(routes::skills::set_auto_review_config),
        )
        .route(
            "/skills/auto-review/config/reset",
            post(routes::skills::reset_auto_review_config),
        )
        .route(
            "/skills/auto-review/recent-rejects",
            get(routes::skills::get_auto_review_recent_rejects),
        )
        .route(
            "/skills/curator/run",
            post(routes::skills::run_skills_curator_now),
        )
        .route(
            "/skills/curator/apply",
            post(routes::skills::apply_skills_curator_merge),
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
        .route(
            "/skills/preset-sources",
            get(routes::skills::discover_preset_skill_sources),
        )
        .route("/skills/{name}", get(routes::skills::get_skill_detail))
        .route("/skills/{name}/toggle", post(routes::skills::toggle_skill))
        .route(
            "/skills/{name}/install",
            post(routes::skills::install_skill_dependency),
        )
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
        .route(
            "/channel/accounts/{id}/auto-transcribe",
            put(routes::channel::set_auto_transcribe_voice),
        )
        .route("/channel/health", get(routes::channel::health_all))
        .route(
            "/channel/sync-commands",
            post(routes::channel::sync_commands),
        )
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
        .route("/channel/handover", post(routes::channel::handover))
        // Crash / Backup
        .route(
            "/crash/recovery-info",
            get(routes::crash::get_crash_recovery_info),
        )
        .route(
            "/settings/config-health",
            get(routes::crash::get_config_health),
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
            "/url-preview/favicon",
            post(routes::url_preview::fetch_url_favicon),
        )
        .route(
            "/url-preview/batch",
            post(routes::url_preview::fetch_url_previews),
        )
        // Embedded browser
        .route("/browser/status", get(routes::browser::get_status))
        .route(
            "/browser/extension/status",
            get(routes::browser::extension_status),
        )
        .route(
            "/browser/extension/install-native-host",
            post(routes::browser::install_native_host_manifest),
        )
        .route(
            "/browser/extension/stop-control",
            post(routes::browser::stop_extension_control),
        )
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
        .route(
            "/browser/capture-frame",
            post(routes::browser::capture_frame),
        )
        .route(
            "/browser/spawn-user-chrome",
            post(routes::browser::spawn_user_chrome),
        )
        .route("/browser/doctor", get(routes::browser::doctor))
        .route(
            "/browser/config",
            get(routes::browser::get_config).post(routes::browser::set_config),
        )
        .route(
            "/browser/install-chromium-runtime",
            post(routes::browser::install_chromium_runtime),
        )
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
        .route(
            "/teams",
            get(routes::team::list_teams).post(routes::team::create_team),
        )
        .route("/teams/{id}", get(routes::team::get_team))
        .route("/teams/{id}/members", get(routes::team::get_team_members))
        .route(
            "/teams/{id}/messages",
            get(routes::team::get_team_messages).post(routes::team::send_user_team_message),
        )
        .route(
            "/teams/{id}/messages/before",
            get(routes::team::get_team_messages_before),
        )
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
        .route(
            "/canvas/by-session/{session_id}",
            get(routes::canvas::list_canvas_projects_by_session),
        )
        // Canvas project CRUD (mirror of Tauri commands).
        .route(
            "/canvas/projects",
            get(routes::canvas::list_canvas_projects),
        )
        .route(
            "/canvas/projects/{project_id}",
            get(routes::canvas::get_canvas_project).delete(routes::canvas::delete_canvas_project),
        )
        // Canvas project static asset tree — serves the iframe's index.html
        // plus its relative CSS / JS / images.
        .route(
            "/canvas/projects/{project_id}/{*rest}",
            get(routes::canvas::serve_canvas_project_file),
        )
        // ── Design Space ──
        .route(
            "/design/projects",
            get(routes::design::list_projects)
                .post(routes::design::create_project)
                .put(routes::design::update_project),
        )
        .route(
            "/design/projects/{id}",
            get(routes::design::get_project).delete(routes::design::delete_project),
        )
        .route(
            "/design/projects/{id}/duplicate",
            post(routes::design::duplicate_project),
        )
        .route(
            "/design/projects/{project_id}/artifacts",
            get(routes::design::list_artifacts),
        )
        .route(
            "/design/projects/{project_id}/chat/thread",
            get(routes::design::chat_thread_latest),
        )
        .route(
            "/design/projects/{project_id}/chat/threads",
            get(routes::design::chat_threads_list),
        )
        .route(
            "/design/artifacts",
            get(routes::design::list_all_artifacts).post(routes::design::create_artifact),
        )
        .route(
            "/design/artifacts/generate",
            // 参考图 base64（「照着这张图做」）可超 axum 默认 2 MiB body 限——放开到 16 MiB。
            post(routes::design::generate_artifact).layer(DefaultBodyLimit::max(16 * 1024 * 1024)),
        )
        .route("/design/patch", post(routes::design::patch_element))
        .route(
            "/design/artifacts/{id}/critique",
            post(routes::design::critique_artifact),
        )
        .route(
            "/design/artifacts/{id}/restyle",
            post(routes::design::restyle_artifact),
        )
        .route(
            "/design/artifacts/{id}",
            get(routes::design::get_artifact).delete(routes::design::delete_artifact),
        )
        .route(
            "/design/artifacts/{id}/versions",
            get(routes::design::list_versions),
        )
        .route(
            "/design/artifacts/{id}/versions/{version}/html",
            get(routes::design::get_version_html),
        )
        .route(
            "/design/artifacts/{id}/share",
            get(routes::design::get_share)
                .post(routes::design::create_share)
                .delete(routes::design::revoke_share),
        )
        .route(
            "/design/deploy/config",
            get(routes::design::get_deploy_config).put(routes::design::save_deploy_config),
        )
        .route(
            "/design/artifacts/{id}/deploy",
            post(routes::design::deploy_artifact),
        )
        .route(
            "/design/artifacts/{id}/export",
            get(routes::design::export_artifact),
        )
        .route(
            "/design/artifacts/{id}/review",
            post(routes::design::review_artifact),
        )
        .route("/design/systems/{id}/kit", get(routes::design::system_kit))
        .route(
            "/design/artifacts/{id}/handoff",
            get(routes::design::export_handoff),
        )
        .route(
            "/design/bindings",
            get(routes::design::list_code_bindings).post(routes::design::bind_code),
        )
        .route(
            "/design/bindings/{id}/sync",
            post(routes::design::sync_code),
        )
        .route(
            "/design/bindings/{id}",
            axum::routing::delete(routes::design::unbind_code),
        )
        .route(
            "/design/artifacts/{id}/restore",
            post(routes::design::restore_version),
        )
        .route(
            "/design/artifacts/{id}/comments",
            get(routes::design::list_comments).post(routes::design::add_comment),
        )
        .route(
            "/design/artifacts/{id}/comments/{comment_id}",
            put(routes::design::update_comment).delete(routes::design::delete_comment),
        )
        .route(
            "/design/artifacts/{id}/comments/{comment_id}/relocate",
            post(routes::design::relocate_comment),
        )
        .route(
            "/design/artifacts/{id}/comments/{comment_id}/resolve",
            post(routes::design::resolve_comment),
        )
        .route(
            "/design/artifacts/{id}/comments/{comment_id}/refine",
            post(routes::design::refine_comment),
        )
        .route("/design/pptx", post(routes::design::export_pptx))
        .route("/design/zip", post(routes::design::export_zip))
        .route(
            "/design/systems",
            get(routes::design::list_systems).post(routes::design::save_system),
        )
        .route(
            "/design/systems/extract",
            post(routes::design::extract_system),
        )
        .route(
            "/design/systems/import",
            post(routes::design::import_design_md),
        )
        .route(
            "/design/systems/figma",
            post(routes::design::import_figma_system),
        )
        .route(
            "/design/directions",
            post(routes::design::propose_directions),
        )
        .route("/design/recipes", get(routes::design::list_recipes))
        .route(
            "/design/artifacts/{id}/native",
            get(routes::design::export_native),
        )
        .route("/design/ffmpeg/doctor", get(routes::design::ffmpeg_doctor))
        .route(
            "/design/ffmpeg/install",
            post(routes::design::install_ffmpeg),
        )
        .route(
            "/design/browser/doctor",
            get(routes::design::browser_doctor),
        )
        .route(
            "/design/browser/install",
            post(routes::design::install_browser),
        )
        .route(
            "/design/systems/{id}/design-md",
            get(routes::design::export_design_md),
        )
        .route(
            "/design/systems/{id}/tokens/export",
            get(routes::design::export_design_tokens),
        )
        .route(
            "/design/systems/{id}",
            get(routes::design::get_system).delete(routes::design::delete_system),
        )
        // Design artifact static asset tree — serves the preview iframe's
        // index.html plus its relative CSS / JS / images.
        .route(
            "/design/projects/{project_id}/artifacts/{artifact_id}/{*rest}",
            get(routes::design::serve_artifact_file),
        )
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
        // Security
        .route(
            "/security/dangerous-status",
            get(routes::misc::dangerous_mode_status),
        )
        .route(
            "/security/dangerous-skip-all-approvals",
            post(routes::misc::set_dangerous_skip_all_approvals),
        )
        // Permission system v2 — pattern lists + Smart mode + Global YOLO status
        .route(
            "/permission/protected-paths",
            get(routes::permission::get_protected_paths)
                .post(routes::permission::set_protected_paths),
        )
        .route(
            "/permission/protected-paths/reset",
            post(routes::permission::reset_protected_paths),
        )
        .route(
            "/permission/dangerous-commands",
            get(routes::permission::get_dangerous_commands)
                .post(routes::permission::set_dangerous_commands),
        )
        .route(
            "/permission/dangerous-commands/reset",
            post(routes::permission::reset_dangerous_commands),
        )
        .route(
            "/permission/edit-commands",
            get(routes::permission::get_edit_commands).post(routes::permission::set_edit_commands),
        )
        .route(
            "/permission/edit-commands/reset",
            post(routes::permission::reset_edit_commands),
        )
        .route(
            "/permission/smart",
            get(routes::permission::get_smart_mode_config)
                .post(routes::permission::set_smart_mode_config),
        )
        .route(
            "/permission/global-yolo",
            get(routes::permission::get_global_yolo_status),
        )
        // macOS control readiness (server/headless returns supported=false)
        .route("/mac-control/status", get(routes::mac_control::status))
        .route(
            "/mac-control/permissions",
            get(routes::mac_control::permissions),
        )
        .route("/mac-control/snapshot", post(routes::mac_control::snapshot))
        .route("/mac-control/elements", post(routes::mac_control::elements))
        .route(
            "/mac-control/capture-frame",
            post(routes::mac_control::capture_frame),
        )
        // Local LLM assistant
        .route("/local-llm/hardware", get(routes::local_llm::get_hardware))
        .route(
            "/local-llm/recommendation",
            get(routes::local_llm::get_recommendation),
        )
        .route(
            "/local-llm/chat-catalog",
            get(routes::local_llm::get_chat_catalog),
        )
        .route(
            "/local-llm/ollama-status",
            get(routes::local_llm::get_ollama_status),
        )
        .route(
            "/local-llm/ollama-version",
            get(routes::local_llm::get_ollama_version),
        )
        .route(
            "/local-llm/known-backends",
            get(routes::local_llm::get_known_backends),
        )
        .route("/local-llm/start", post(routes::local_llm::start))
        .route("/local-llm/models", get(routes::local_llm::list_models))
        .route(
            "/local-llm/library/search",
            get(routes::local_llm::search_library),
        )
        .route(
            "/local-llm/library/model",
            post(routes::local_llm::get_library_model),
        )
        .route("/local-llm/preload", post(routes::local_llm::preload))
        .route("/local-llm/stop-model", post(routes::local_llm::stop_model))
        .route(
            "/local-llm/delete-model",
            post(routes::local_llm::delete_model),
        )
        .route(
            "/local-llm/provider-model",
            post(routes::local_llm::add_provider_model),
        )
        .route(
            "/local-llm/default-model",
            post(routes::local_llm::set_default_model),
        )
        .route(
            "/local-llm/embedding-config",
            post(routes::local_llm::add_embedding_config),
        )
        // SearXNG Docker
        .route("/searxng/status", get(routes::searxng::status))
        .route("/searxng/deploy", post(routes::searxng::deploy))
        .route("/searxng/start", post(routes::searxng::start))
        .route("/searxng/stop", post(routes::searxng::stop))
        .route("/searxng", delete(routes::searxng::remove))
        // Auth
        .route("/auth/codex/start", post(routes::auth::start_codex_auth))
        .route(
            "/auth/codex/finalize",
            post(routes::auth::finalize_codex_auth),
        )
        .route("/auth/codex/status", get(routes::auth::check_auth_status))
        .route("/auth/codex/logout", post(routes::auth::logout_codex))
        .route("/auth/codex/models", get(routes::auth::get_codex_models))
        .route("/auth/codex/models", post(routes::auth::set_codex_model))
        .route(
            "/auth/session/restore",
            post(routes::auth::try_restore_session),
        )
        // System (desktop-only stubs)
        .route("/system/restart", post(routes::system::request_app_restart))
        .route("/system/timezone", get(routes::system::get_system_timezone))
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
        // Filesystem (server-side directory browser for the working-dir picker
        // and the chat-input `@` mention popper)
        .route("/filesystem/list-dir", get(routes::filesystem::list_dir))
        .route(
            "/filesystem/create-dir",
            post(routes::filesystem::create_dir),
        )
        .route(
            "/filesystem/search-files",
            get(routes::filesystem::search_files),
        )
        // Project file browser (workspace-scoped). Reads are always available;
        // writes are gated by `filesystem.allow_remote_writes` in the handlers.
        .route("/fs/list", get(routes::project_fs::fs_list))
        .route("/fs/read", get(routes::project_fs::fs_read))
        .route("/fs/extract", get(routes::project_fs::fs_extract))
        .route("/fs/search", get(routes::project_fs::fs_search))
        .route("/fs/raw", get(routes::project_fs::fs_raw))
        .route("/fs/git", get(routes::project_fs::fs_git_info))
        // Raise the body cap above axum's 2MB default so saving a file as large
        // as the read-preview ceiling (5MB) isn't rejected before the handler.
        .route(
            "/fs/file",
            put(routes::project_fs::fs_write).layer(DefaultBodyLimit::max(8 * 1024 * 1024)),
        )
        .route("/fs/entry", delete(routes::project_fs::fs_delete))
        .route("/fs/rename", post(routes::project_fs::fs_rename))
        .route("/fs/mkdir", post(routes::project_fs::fs_mkdir))
        .route(
            "/fs/upload",
            post(routes::project_fs::fs_upload).layer(DefaultBodyLimit::max(25 * 1024 * 1024)),
        )
        // Dev tools
        .route("/dev/clear-sessions", post(routes::dev::clear_sessions))
        .route("/dev/clear-cron", post(routes::dev::clear_cron))
        .route("/dev/clear-memory", post(routes::dev::clear_memory))
        .route("/dev/reset-config", post(routes::dev::reset_config))
        .route("/dev/clear-all", post(routes::dev::clear_all))
        // STT subsystem
        .route(
            "/stt/providers",
            get(routes::stt::list_stt_providers).post(routes::stt::add_stt_provider),
        )
        .route(
            "/stt/providers/{id}",
            put(routes::stt::update_stt_provider).delete(routes::stt::delete_stt_provider),
        )
        .route(
            "/stt/providers/reorder",
            post(routes::stt::reorder_stt_providers),
        )
        .route(
            "/stt/active-model",
            get(routes::stt::get_active_stt_model)
                .put(routes::stt::set_active_stt_model)
                .delete(routes::stt::clear_active_stt_model),
        )
        .route(
            "/stt/fallback-models",
            get(routes::stt::get_stt_fallback_models).put(routes::stt::set_stt_fallback_models),
        )
        .route(
            "/stt/im-fallback-model",
            get(routes::stt::get_im_fallback_stt_model).put(routes::stt::set_im_fallback_stt_model),
        )
        .route(
            "/stt/local-backends",
            get(routes::stt::list_local_stt_backends),
        )
        .route(
            "/stt/local-backends/{key}/probe",
            get(routes::stt::probe_local_stt_backend),
        )
        .route(
            "/stt/local-backends/{backendKey}/upsert",
            post(routes::stt::upsert_local_stt_provider),
        )
        .route(
            "/stt/transcribe",
            post(routes::stt::stt_transcribe_blob)
                // base64-encoded audio is ~4/3 of the decoded byte cap;
                // round up to leave slop for JSON metadata + envelope.
                .layer(DefaultBodyLimit::max(
                    (ha_core::stt::MAX_BATCH_AUDIO_BYTES * 4 / 3) + 1024 * 1024,
                )),
        )
        .route("/stt/sessions", post(routes::stt::stt_start_session))
        .route(
            "/stt/sessions/{id}/chunk",
            post(routes::stt::stt_push_chunk)
                // Per-chunk uplink — match the in-memory `MAX_WS_FRAME_BYTES`
                // (1 MiB) deepgram is willing to accept.
                .layer(DefaultBodyLimit::max(2 * 1024 * 1024)),
        )
        .route(
            "/stt/sessions/{id}/finalize",
            post(routes::stt::stt_finalize_session),
        )
        .route(
            "/stt/sessions/{id}",
            delete(routes::stt::stt_cancel_session),
        );

    let ws_routes = Router::new().route("/events", get(ws::events::events_ws));

    // Apply API key auth middleware to protected routes
    let auth_state = middleware::ApiKeyState {
        api_key,
        knowledge_agent_read_token,
    };
    let protected = Router::new()
        .nest("/api", api)
        .nest("/ws", ws_routes)
        .route_layer(axum::middleware::from_fn_with_state(
            auth_state,
            middleware::require_api_key,
        ));

    let base = Router::new().merge(health).merge(protected);

    attach_web_fallback(base)
        .layer(build_cors_layer(cors_origins))
        .layer(axum::middleware::from_fn(middleware::access_log))
        .with_state(ctx)
}

/// Attach the Web GUI static-file fallback to the given router.
///
/// Any request that doesn't match `/api/*`, `/ws/*`, or another
/// already-registered route falls through to the front-end bundle so
/// users can open `http://host:port/` in a browser and get the React UI.
fn attach_web_fallback(router: Router<Arc<AppContext>>) -> Router<Arc<AppContext>> {
    match web_assets::resolve_strategy() {
        web_assets::WebAssetStrategy::ServeDir(path) => {
            let index = path.join("index.html");
            let serve = tower_http::services::ServeDir::new(&path)
                .fallback(tower_http::services::ServeFile::new(index));
            router.fallback_service(serve)
        }
        web_assets::WebAssetStrategy::Embedded => router.fallback(web_assets::serve_embedded),
        web_assets::WebAssetStrategy::Unavailable => {
            router.fallback(web_assets::serve_unavailable_notice)
        }
    }
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
