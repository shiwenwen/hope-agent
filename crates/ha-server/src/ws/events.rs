use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use std::sync::Arc;

use ha_core::chat_engine::stream_broadcast::{EVENT_CHANNEL_STREAM_DELTA, EVENT_CHAT_STREAM_DELTA};

use crate::AppContext;

/// `WS /ws/events` — subscribes to the EventBus and forwards all `AppEvent`
/// as JSON text frames to the client.
///
/// Each WebSocket connection gets its own broadcast `Receiver`, so multiple
/// clients can independently consume events.
pub async fn events_ws(
    ws: WebSocketUpgrade,
    State(ctx): State<Arc<AppContext>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_events_socket(socket, ctx))
}

/// Send timeout — disconnect clients that can't keep up.
const SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// Max consecutive lag events before disconnecting.
const MAX_LAG_COUNT: u32 = 3;

async fn handle_events_socket(mut socket: WebSocket, ctx: Arc<AppContext>) {
    use futures_util::SinkExt;
    use tokio::sync::broadcast::error::RecvError;

    let mut rx = ctx.event_bus.subscribe();
    let mut lag_count: u32 = 0;
    let api_key = ctx.api_key.clone();

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        lag_count = 0;
                        // Only chat/channel stream deltas carry nested `payload.event`
                        // strings with `media_items` that need `localPath` stripped
                        // and `?token=` stamped. Everything else (session events,
                        // logging, approvals…) skips the extra Value round-trip.
                        let name = event.name.as_str();
                        let json = if name == EVENT_CHAT_STREAM_DELTA
                            || name == EVENT_CHANNEL_STREAM_DELTA
                        {
                            let mut event_val = match serde_json::to_value(&event) {
                                Ok(v) => v,
                                Err(_) => continue,
                            };
                            ha_core::agent::rewrite_envelope_event_for_http(
                                &mut event_val,
                                api_key.as_deref(),
                            );
                            match serde_json::to_string(&event_val) {
                                Ok(j) => j,
                                Err(_) => continue,
                            }
                        } else {
                            match serde_json::to_string(&event) {
                                Ok(j) => j,
                                Err(_) => continue,
                            }
                        };
                        // Disconnect slow clients instead of blocking the event loop.
                        let send_result = tokio::time::timeout(
                            SEND_TIMEOUT,
                            socket.send(Message::Text(json.into())),
                        ).await;
                        match send_result {
                            Ok(Err(_)) | Err(_) => break, // send error or timeout
                            Ok(Ok(())) => {}
                        }
                    }
                    Err(RecvError::Lagged(n)) => {
                        lag_count += 1;
                        if lag_count >= MAX_LAG_COUNT {
                            // Persistently slow client — disconnect.
                            break;
                        }
                        let msg = serde_json::json!({
                            "name": "_lagged",
                            "payload": { "missed": n },
                        });
                        let _ = socket.send(Message::Text(msg.to_string().into())).await;
                    }
                    Err(RecvError::Closed) => break,
                }
            }

            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }

    let _ = socket.close().await;
}
