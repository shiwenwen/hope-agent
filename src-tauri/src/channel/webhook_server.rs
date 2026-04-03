//! Embedded HTTP server for receiving webhooks from Google Chat, LINE, etc.
//!
//! Starts lazily when the first webhook-based channel account starts,
//! and stops when the last one stops. Binds to 127.0.0.1:<port>.
//! Users must configure a tunnel (ngrok, cloudflared) for public access.

use anyhow::Result;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};

/// Default port for the webhook server (avoids conflict with OAuth callback on 1455).
pub const DEFAULT_WEBHOOK_PORT: u16 = 1456;

/// A webhook handler receives the raw request body and headers, returns a response body.
pub type WebhookHandlerFn = Arc<
    dyn Fn(WebhookRequest) -> std::pin::Pin<Box<dyn std::future::Future<Output = WebhookResponse> + Send>>
        + Send
        + Sync,
>;

/// Incoming webhook request data.
pub struct WebhookRequest {
    pub body: Vec<u8>,
    pub headers: HashMap<String, String>,
    pub path_params: (String, String), // (channel, account_id)
}

/// Response from a webhook handler.
pub struct WebhookResponse {
    pub status: u16,
    pub body: String,
}

/// Shared state for the webhook server, holding registered handlers.
struct WebhookState {
    /// Map of "channel/account_id" → handler function
    handlers: Mutex<HashMap<String, WebhookHandlerFn>>,
    /// Channel to forward unmatched requests for logging
    _log_tx: mpsc::Sender<String>,
}

/// The webhook server instance.
pub struct WebhookServer {
    port: u16,
    state: Arc<WebhookState>,
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
}

impl WebhookServer {
    /// Create and start the webhook server on the given port.
    pub async fn start(port: u16) -> Result<Arc<Self>> {
        let (log_tx, mut log_rx) = mpsc::channel(64);

        let state = Arc::new(WebhookState {
            handlers: Mutex::new(HashMap::new()),
            _log_tx: log_tx,
        });

        let app_state = state.clone();
        let app = Router::new()
            .route("/webhook/{channel}/{account_id}", post(handle_webhook))
            .with_state(app_state);

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port)).await?;

        app_info!(
            "webhook",
            "server",
            "Webhook server started on 127.0.0.1:{}",
            port
        );

        // Spawn the server
        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .ok();
        });

        // Spawn log consumer (just discard for now)
        tokio::spawn(async move {
            while let Some(_msg) = log_rx.recv().await {
                // Could log unmatched webhooks here
            }
        });

        Ok(Arc::new(Self {
            port,
            state,
            shutdown_tx: Mutex::new(Some(shutdown_tx)),
        }))
    }

    /// Register a webhook handler for a specific channel and account.
    pub async fn register_handler(
        &self,
        channel: &str,
        account_id: &str,
        handler: WebhookHandlerFn,
    ) {
        let key = format!("{}/{}", channel, account_id);
        let mut handlers = self.state.handlers.lock().await;
        handlers.insert(key, handler);
    }

    /// Unregister a webhook handler.
    pub async fn unregister_handler(&self, channel: &str, account_id: &str) {
        let key = format!("{}/{}", channel, account_id);
        let mut handlers = self.state.handlers.lock().await;
        handlers.remove(&key);
    }

    /// Check if any handlers are registered.
    pub async fn has_handlers(&self) -> bool {
        let handlers = self.state.handlers.lock().await;
        !handlers.is_empty()
    }

    /// Get the local URL for this server.
    pub fn local_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Stop the webhook server.
    pub async fn stop(&self) {
        let mut tx = self.shutdown_tx.lock().await;
        if let Some(tx) = tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Axum handler for incoming webhooks.
async fn handle_webhook(
    Path((channel, account_id)): Path<(String, String)>,
    State(state): State<Arc<WebhookState>>,
    request: Request<Body>,
) -> impl IntoResponse {
    let key = format!("{}/{}", channel, account_id);

    let handler = {
        let handlers = state.handlers.lock().await;
        handlers.get(&key).cloned()
    };

    let Some(handler) = handler else {
        return (StatusCode::NOT_FOUND, "No handler registered".to_string());
    };

    // Extract headers
    let mut headers = HashMap::new();
    for (name, value) in request.headers() {
        if let Ok(v) = value.to_str() {
            headers.insert(name.to_string(), v.to_string());
        }
    }

    // Read body
    let body = match axum::body::to_bytes(request.into_body(), 1024 * 1024).await {
        Ok(b) => b.to_vec(),
        Err(_) => return (StatusCode::BAD_REQUEST, "Failed to read body".to_string()),
    };

    let req = WebhookRequest {
        body,
        headers,
        path_params: (channel, account_id),
    };

    let resp = handler(req).await;
    (
        StatusCode::from_u16(resp.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        resp.body,
    )
}
