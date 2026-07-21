use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::SinkExt;
use std::sync::Arc;

use ha_core::chat_engine::stream_broadcast::{EVENT_CHANNEL_STREAM_DELTA, EVENT_CHAT_STREAM_DELTA};
use ha_core::event_bus::AppEvent;

use crate::AppContext;

/// `WS /ws/events` — subscribes to the app and terminal event streams and
/// forwards authorized events as JSON text frames to the client.
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

fn remote_terminal_access_allowed() -> bool {
    ha_core::config::cached_config()
        .filesystem
        .allow_remote_writes
}

fn event_json_for_http(event: &AppEvent, api_key: Option<&str>) -> Option<String> {
    // Only chat/channel stream deltas carry nested `payload.event` strings
    // with `media_items` that need `localPath` stripped and `?token=` stamped.
    let name = event.name.as_str();
    if name == EVENT_CHAT_STREAM_DELTA || name == EVENT_CHANNEL_STREAM_DELTA {
        let mut event_val = serde_json::to_value(event).ok()?;
        ha_core::agent::rewrite_envelope_event_for_http(&mut event_val, api_key);
        serde_json::to_string(&event_val).ok()
    } else {
        serde_json::to_string(event).ok()
    }
}

async fn send_event(socket: &mut WebSocket, event: &AppEvent, api_key: Option<&str>) -> bool {
    let Some(json) = event_json_for_http(event, api_key) else {
        return true;
    };
    matches!(
        tokio::time::timeout(SEND_TIMEOUT, socket.send(Message::Text(json.into()))).await,
        Ok(Ok(()))
    )
}

async fn send_lag_notice(socket: &mut WebSocket, missed: u64, stream: &str) -> bool {
    let msg = serde_json::json!({
        "name": "_lagged",
        "payload": { "missed": missed, "stream": stream },
    });
    matches!(
        tokio::time::timeout(
            SEND_TIMEOUT,
            socket.send(Message::Text(msg.to_string().into())),
        )
        .await,
        Ok(Ok(()))
    )
}

async fn handle_events_socket(mut socket: WebSocket, ctx: Arc<AppContext>) {
    use tokio::sync::broadcast::error::RecvError;

    let _conn_guard =
        ha_core::server_status::WsConnectionGuard::new(ha_core::server_status::events_ws_counter());

    let mut app_rx = ctx.event_bus.subscribe();
    let mut terminal_rx = ctx.terminal_manager.subscribe_output_events();
    let mut app_lag_count: u32 = 0;
    let mut terminal_lag_count: u32 = 0;
    let api_key = ctx.api_key.clone();

    loop {
        tokio::select! {
            result = app_rx.recv() => {
                match result {
                    Ok(event) => {
                        app_lag_count = 0;
                        if ha_core::terminal::is_terminal_event_name(&event.name)
                            && !remote_terminal_access_allowed()
                        {
                            continue;
                        }
                        if !send_event(&mut socket, &event, api_key.as_deref()).await {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(n)) => {
                        app_lag_count += 1;
                        if app_lag_count >= MAX_LAG_COUNT {
                            break;
                        }
                        if !send_lag_notice(&mut socket, n, "app").await {
                            break;
                        }
                    }
                    Err(RecvError::Closed) => break,
                }
            }

            result = terminal_rx.recv() => {
                match result {
                    Ok(event) => {
                        terminal_lag_count = 0;
                        if !remote_terminal_access_allowed() {
                            continue;
                        }
                        if !send_event(&mut socket, &event, api_key.as_deref()).await {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(n)) => {
                        if !remote_terminal_access_allowed() {
                            terminal_lag_count = 0;
                            continue;
                        }
                        terminal_lag_count += 1;
                        if terminal_lag_count >= MAX_LAG_COUNT {
                            break;
                        }
                        if !send_lag_notice(&mut socket, n, "terminal").await {
                            break;
                        }
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
