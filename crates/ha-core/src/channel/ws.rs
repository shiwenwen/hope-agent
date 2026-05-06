use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{self, http::Request, Message},
    MaybeTlsStream, WebSocketStream,
};

/// A thin wrapper around a tokio-tungstenite WebSocket connection.
pub struct WsConnection {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

/// Close frame metadata exposed by [`WsConnection::recv_text_with_close`].
#[derive(Debug, Clone)]
pub struct WsClose {
    /// Close code per RFC 6455 + protocol-specific extensions (Discord 4xxx,
    /// QQ Bot 4xxx etc.). Tungstenite's `CloseCode` enum is flattened to u16
    /// so unknown extensions can still be matched.
    pub code: u16,
    /// Optional reason string sent by the peer.
    pub reason: String,
}

impl WsConnection {
    /// Connect to a WebSocket URL.
    pub async fn connect(url: &str) -> Result<Self> {
        let (ws, _resp) = connect_async(url)
            .await
            .map_err(|e| anyhow!("WebSocket connect failed: {}", e))?;
        Ok(Self { ws })
    }

    /// Connect with custom HTTP headers (e.g. Authorization).
    pub async fn connect_with_headers(url: &str, headers: Vec<(&str, &str)>) -> Result<Self> {
        let mut builder = Request::builder().uri(url);
        for (key, value) in headers {
            builder = builder.header(key, value);
        }
        let req = builder
            .body(())
            .map_err(|e| anyhow!("Failed to build request: {}", e))?;
        let (ws, _resp) = connect_async(req)
            .await
            .map_err(|e| anyhow!("WebSocket connect failed: {}", e))?;
        Ok(Self { ws })
    }

    /// Send a JSON-serializable value as a text message.
    pub async fn send_json(&mut self, value: &impl serde::Serialize) -> Result<()> {
        let text = serde_json::to_string(value)?;
        self.ws
            .send(Message::Text(text.into()))
            .await
            .map_err(|e| anyhow!("WebSocket send failed: {}", e))
    }

    /// Send a raw text message.
    pub async fn send_text(&mut self, text: String) -> Result<()> {
        self.ws
            .send(Message::Text(text.into()))
            .await
            .map_err(|e| anyhow!("WebSocket send failed: {}", e))
    }

    /// Send a raw binary message.
    pub async fn send_binary(&mut self, bytes: Vec<u8>) -> Result<()> {
        self.ws
            .send(Message::Binary(bytes.into()))
            .await
            .map_err(|e| anyhow!("WebSocket send failed: {}", e))
    }

    /// Receive the next text message, returning None on close/error.
    ///
    /// Close frames are swallowed; callers that need to inspect the close code
    /// (e.g. Discord 4xxx fatal vs. resumable) should use
    /// [`recv_text_with_close`](Self::recv_text_with_close) instead.
    pub async fn recv_text(&mut self) -> Option<String> {
        loop {
            match self.ws.next().await {
                Some(Ok(Message::Text(text))) => return Some(text.to_string()),
                Some(Ok(Message::Close(_))) => return None,
                Some(Ok(Message::Ping(data))) => {
                    let _ = self.ws.send(Message::Pong(data)).await;
                    continue;
                }
                Some(Ok(_)) => continue, // Binary, Pong, Frame — skip
                Some(Err(_)) => return None,
                None => return None,
            }
        }
    }

    /// Receive the next text message, exposing close frame metadata.
    ///
    /// Returns:
    /// - `Some(Ok(text))` — normal data frame
    /// - `Some(Err(WsClose { code, reason }))` — peer closed with a frame; caller
    ///   uses `code` to route fatal vs. resumable (Discord 4004/4010-4014 fatal,
    ///   4007/4009 fresh IDENTIFY, etc.)
    /// - `None` — IO error or stream end without a close frame
    pub async fn recv_text_with_close(&mut self) -> Option<Result<String, WsClose>> {
        loop {
            match self.ws.next().await {
                Some(Ok(Message::Text(text))) => return Some(Ok(text.to_string())),
                Some(Ok(Message::Close(frame))) => {
                    let close = match frame {
                        Some(f) => WsClose {
                            code: u16::from(f.code),
                            reason: f.reason.to_string(),
                        },
                        None => WsClose {
                            code: 1005, // No Status Received
                            reason: String::new(),
                        },
                    };
                    return Some(Err(close));
                }
                Some(Ok(Message::Ping(data))) => {
                    let _ = self.ws.send(Message::Pong(data)).await;
                    continue;
                }
                Some(Ok(_)) => continue, // Binary, Pong, Frame — skip
                Some(Err(_)) => return None,
                None => return None,
            }
        }
    }

    /// Receive the next binary message, transparently echoing tungstenite-level
    /// ping/pong frames. Returns None on close/error.
    pub async fn recv_binary(&mut self) -> Option<Vec<u8>> {
        loop {
            match self.ws.next().await {
                Some(Ok(Message::Binary(bytes))) => return Some(bytes.to_vec()),
                Some(Ok(Message::Close(_))) => return None,
                Some(Ok(Message::Ping(data))) => {
                    let _ = self.ws.send(Message::Pong(data)).await;
                    continue;
                }
                Some(Ok(_)) => continue, // Text, Pong, Frame — skip
                Some(Err(_)) => return None,
                None => return None,
            }
        }
    }

    /// Close the WebSocket connection gracefully.
    pub async fn close(&mut self) {
        let _ = self
            .ws
            .close(Some(tungstenite::protocol::CloseFrame {
                code: tungstenite::protocol::frame::coding::CloseCode::Normal,
                reason: "shutdown".into(),
            }))
            .await;
    }
}

/// Reconnect backoff delays in seconds: [1, 2, 5, 10, 30, 60].
pub const BACKOFF_SECS: &[u64] = &[1, 2, 5, 10, 30, 60];

/// Get the backoff duration for a given attempt (0-indexed), capping at the last value.
pub fn backoff_duration(attempt: usize) -> std::time::Duration {
    let secs = BACKOFF_SECS
        .get(attempt)
        .copied()
        .unwrap_or_else(|| BACKOFF_SECS.last().copied().unwrap_or(60));
    std::time::Duration::from_secs(secs)
}
