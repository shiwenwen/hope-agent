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

    /// Receive the next text message, returning None on close/error.
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
