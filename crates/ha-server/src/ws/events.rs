use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::Extension;
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
    Extension(auth): Extension<crate::middleware::AuthState>,
    headers: HeaderMap,
) -> Response {
    let owner_changes = auth.subscribe_owner_changes();
    let protocol = crate::middleware::event_ticket_protocol(&headers).filter(|protocol| {
        protocol
            .strip_prefix(crate::middleware::EVENT_TICKET_PROTOCOL_PREFIX)
            .is_some_and(|ticket| auth.check_access_ticket(ticket, "events"))
    });
    // Authentication was already checked by middleware. Revalidate after
    // subscribing to changes to close the race where a token rotates between
    // middleware and the WebSocket upgrade.
    if auth.auth_required() && protocol.is_none() && !auth.headers_are_owner_authenticated(&headers)
    {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    }
    let ws = match protocol {
        Some(ref protocol) => ws.protocols([protocol.clone()]),
        None => ws,
    };
    ws.on_upgrade(move |socket| {
        handle_events_socket(socket, ctx, auth, headers, protocol, owner_changes)
    })
    .into_response()
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

fn event_json_for_http(event: &AppEvent) -> Option<String> {
    // Only chat/channel stream deltas carry nested `payload.event` strings
    // with `media_items` that need server-local paths stripped.
    let name = event.name.as_str();
    if name == EVENT_CHAT_STREAM_DELTA || name == EVENT_CHANNEL_STREAM_DELTA {
        let mut event_val = serde_json::to_value(event).ok()?;
        ha_core::agent::rewrite_envelope_event_for_http(&mut event_val);
        serde_json::to_string(&event_val).ok()
    } else {
        serde_json::to_string(event).ok()
    }
}

async fn send_event(socket: &mut WebSocket, event: &AppEvent) -> bool {
    let Some(json) = event_json_for_http(event) else {
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

async fn handle_events_socket(
    mut socket: WebSocket,
    ctx: Arc<AppContext>,
    auth: crate::middleware::AuthState,
    headers: HeaderMap,
    event_protocol: Option<String>,
    mut owner_changes: tokio::sync::watch::Receiver<u64>,
) {
    use tokio::sync::broadcast::error::RecvError;

    let _conn_guard =
        ha_core::server_status::WsConnectionGuard::new(ha_core::server_status::events_ws_counter());

    let mut app_rx = ctx.event_bus.subscribe();
    let mut terminal_rx = ctx.terminal_manager.subscribe_output_events();
    let mut app_lag_count: u32 = 0;
    let mut terminal_lag_count: u32 = 0;
    let mut credential_check = tokio::time::interval_at(
        tokio::time::Instant::now() + std::time::Duration::from_secs(30),
        std::time::Duration::from_secs(30),
    );

    loop {
        tokio::select! {
            changed = owner_changes.changed() => {
                // Token replacement revokes both browser sessions and scoped
                // tickets. Close the already-upgraded stream as well; active
                // clients will reconnect using the new credential.
                let _ = changed;
                break;
            }
            _ = credential_check.tick() => {
                let ticket_is_valid = event_protocol.as_deref().is_some_and(|protocol| {
                    protocol
                        .strip_prefix(crate::middleware::EVENT_TICKET_PROTOCOL_PREFIX)
                        .is_some_and(|ticket| auth.check_access_ticket(ticket, "events"))
                });
                if auth.auth_required()
                    && !ticket_is_valid
                    && !auth.headers_are_owner_authenticated(&headers)
                {
                    // WebSocket admission credentials have finite lifetimes.
                    // Revalidate them so a connection cannot outlive an
                    // expired scoped ticket or browser session indefinitely.
                    break;
                }
            }
            result = app_rx.recv() => {
                match result {
                    Ok(event) => {
                        app_lag_count = 0;
                        if ha_core::terminal::is_terminal_event_name(&event.name)
                            && !remote_terminal_access_allowed()
                        {
                            continue;
                        }
                        if !send_event(&mut socket, &event).await {
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
                        if !send_event(&mut socket, &event).await {
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
