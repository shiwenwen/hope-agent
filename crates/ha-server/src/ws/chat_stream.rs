use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::SinkExt;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

use crate::AppContext;

// ── Per-session broadcast registry ─────────────────────────────

/// Registry of per-session broadcast channels for chat streaming.
/// When `POST /api/chat` runs, it broadcasts events here; connected
/// WebSocket clients receive them in real time.
pub struct ChatStreamRegistry {
    /// Map from session_id to broadcast sender.
    sessions: RwLock<HashMap<String, broadcast::Sender<String>>>,
}

impl Default for ChatStreamRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatStreamRegistry {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
        }
    }

    /// Get or create a broadcast channel for a session.
    /// Returns a receiver for new subscribers.
    pub async fn subscribe(&self, session_id: &str) -> broadcast::Receiver<String> {
        let mut map = self.sessions.write().await;
        let tx = map.entry(session_id.to_string()).or_insert_with(|| {
            let (tx, _) = broadcast::channel(256);
            tx
        });
        tx.subscribe()
    }

    /// Broadcast an event to all WebSocket subscribers for a session.
    /// No-op if no subscribers are connected.
    pub async fn broadcast(&self, session_id: &str, event: &str) {
        let map = self.sessions.read().await;
        if let Some(tx) = map.get(session_id) {
            let _ = tx.send(event.to_string());
        }
    }

    /// Remove a session's broadcast channel when it has no more subscribers.
    pub async fn cleanup(&self, session_id: &str) {
        let mut map = self.sessions.write().await;
        if let Some(tx) = map.get(session_id) {
            // Only remove if no receivers remain
            if tx.receiver_count() == 0 {
                map.remove(session_id);
            }
        }
    }
}

// ── WebSocket handler ──────────────────────────────────────────

/// `WS /ws/chat/{session_id}` — subscribe to chat streaming events for a session.
///
/// When the chat engine runs (via `POST /api/chat`), it broadcasts events
/// through the `ChatStreamRegistry`. Each WebSocket connection receives
/// these events in real time.
pub async fn chat_stream_ws(
    ws: WebSocketUpgrade,
    Path(session_id): Path<String>,
    State(ctx): State<Arc<AppContext>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_chat_socket(socket, session_id, ctx))
}

async fn handle_chat_socket(mut socket: WebSocket, session_id: String, ctx: Arc<AppContext>) {
    let _conn_guard =
        ha_core::server_status::WsConnectionGuard::new(ha_core::server_status::chat_ws_counter());

    let mut rx = ctx.chat_streams.subscribe(&session_id).await;

    // Send connection acknowledgement
    let ack = serde_json::json!({
        "type": "connected",
        "session_id": &session_id,
    });
    if socket
        .send(Message::Text(ack.to_string().into()))
        .await
        .is_err()
    {
        return;
    }

    // Forward broadcast events to WebSocket, also listen for client messages
    loop {
        tokio::select! {
            // Receive events from the chat engine broadcast
            result = rx.recv() => {
                match result {
                    Ok(event_str) => {
                        let send_result = tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            socket.send(Message::Text(event_str.into())),
                        ).await;
                        match send_result {
                            Ok(Err(_)) | Err(_) => break, // send error or timeout
                            Ok(Ok(())) => {}
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        let msg = serde_json::json!({
                            "type": "_lagged",
                            "missed": n,
                        });
                        let _ = socket.send(Message::Text(msg.to_string().into())).await;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }

            // Listen for client messages (close, ping, etc.)
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        let _ = socket.send(Message::Pong(data)).await;
                    }
                    _ => {}
                }
            }
        }
    }

    let _ = socket.close().await;

    // Cleanup the broadcast channel if no more subscribers
    ctx.chat_streams.cleanup(&session_id).await;
}
