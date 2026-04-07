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
        .route("/config/server", get(routes::config::get_server_config))
        .route("/config/server", put(routes::config::save_server_config))
        // Agents
        .route("/agents", get(routes::agents::list_agents))
        .route("/agents/{id}", get(routes::agents::get_agent))
        .route("/agents/{id}", put(routes::agents::save_agent))
        .route("/agents/{id}", delete(routes::agents::delete_agent));

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
