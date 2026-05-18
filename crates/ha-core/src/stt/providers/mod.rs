//! STT provider implementations.
//!
//! Batch paths:
//! - `openai`: multipart `/v1/audio/transcriptions` for OpenAI Whisper,
//!   gpt-4o-transcribe batch, Groq, StepFun, SiliconFlow, whisper.cpp
//!   server, faster-whisper-server, FunASR + OpenAI wrapper, sherpa-onnx
//!   server.
//! - `chat_completions_asr`: chat-completions + `input_audio` content
//!   blocks for providers that ship ASR as a multimodal LLM rather than
//!   a dedicated transcription endpoint. Current target: Alibaba
//!   DashScope (Qwen3-ASR). Same wire shape also applies to OpenAI's
//!   gpt-4o-audio-preview and similar multimodal-LLM ASR offerings.
//!
//! Streaming WS path: Deepgram / AssemblyAI / Azure Speech / iFlytek IAT /
//! Volcengine. Each module exposes `open_stream` returning a `SttStream`
//! handle so [`crate::stt::session::SttSessionManager`] can route by kind
//! without leaking per-provider types.

use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

use super::errors::{SttError, SttResult};
use super::types::{AudioPayload, TranscriptDelta, MAX_BATCH_AUDIO_BYTES};

pub mod assemblyai;
pub mod azure;
pub mod chat_completions_asr;
pub mod deepgram;
pub mod openai;
pub mod volcengine;
pub mod xunfei;

/// WS frame caps shared across every WS provider, matching the
/// MCP client (`mcp/transport.rs`) defaults. Picked to bound the
/// per-message memory a misbehaving / hostile server can force on us.
pub(super) const WS_MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
pub(super) const WS_MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Shared mpsc capacity for the per-provider audio uplink and delta
/// downlink channels. ~6s of buffering at 100ms PCM16/16kHz chunks; any
/// real backpressure surfaces to the caller via `mpsc::Sender::send`.
pub(super) const STT_STREAM_CHANNEL_CAPACITY: usize = 64;

/// Common streaming handle returned by every WS provider's `open_stream`:
/// callers push raw audio bytes into `audio_tx` and drain partial / final
/// transcript deltas from `delta_rx`. Dropping `audio_tx` signals EOS so
/// the upstream task can close the WS gracefully.
pub struct SttStream {
    pub audio_tx: mpsc::Sender<Vec<u8>>,
    pub delta_rx: mpsc::Receiver<Result<TranscriptDelta, SttError>>,
}

/// Derive an http(s) "twin" URL from a ws(s) URL so it can flow through
/// `security::ssrf::check_url` (which rejects ws/wss schemes). All five
/// WS STT providers share the same twin step.
pub(super) fn ws_to_https_twin(url: &str, provider_label: &str) -> SttResult<String> {
    let mut parsed = url::Url::parse(url)
        .map_err(|e| SttError::Other(format!("Invalid {provider_label} URL: {e}")))?;
    let new_scheme = match parsed.scheme() {
        "wss" => Some("https"),
        "ws" => Some("http"),
        _ => None,
    };
    if let Some(scheme) = new_scheme {
        parsed
            .set_scheme(scheme)
            .map_err(|_| SttError::Other("Failed to derive SSRF twin URL".into()))?;
    }
    Ok(parsed.to_string())
}

/// Build a `WebSocketConfig` with the shared `WS_MAX_*_BYTES` caps and
/// connect via `tokio_tungstenite::connect_async_with_config`. Callers
/// build their own `Request` (so they can attach provider-specific
/// headers) and pass it in directly.
pub(super) async fn ws_connect_with_caps<R: IntoClientRequest + Unpin>(
    request: R,
    provider_label: &str,
) -> SttResult<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
> {
    let ws_config = WebSocketConfig::default()
        .max_message_size(Some(WS_MAX_MESSAGE_BYTES))
        .max_frame_size(Some(WS_MAX_FRAME_BYTES));
    let (ws, _resp) = tokio_tungstenite::connect_async_with_config(request, Some(ws_config), false)
        .await
        .map_err(|e| SttError::Network(format!("{provider_label} WS connect failed: {e}")))?;
    Ok(ws)
}

// ── Batch (HTTP) provider shared helpers ─────────────────────────────────

/// Resolve an `AudioPayload` into a `(bytes, mime_type, filename)` tuple,
/// enforcing the shared `MAX_BATCH_AUDIO_BYTES` cap up front (statting
/// `File` payloads before reading so we never alloc the giant Vec). The
/// returned filename is best-effort — `Bytes` payloads carry their own
/// hint, `File` payloads fall back to `"audio.bin"`.
pub(super) async fn load_batch_audio(
    audio: AudioPayload,
) -> SttResult<(Vec<u8>, String, String)> {
    match audio {
        AudioPayload::Bytes {
            bytes,
            mime_type,
            filename,
        } => {
            if bytes.len() > MAX_BATCH_AUDIO_BYTES {
                return Err(SttError::UnsupportedAudio(format!(
                    "Audio payload {} bytes exceeds {} MiB batch limit",
                    bytes.len(),
                    MAX_BATCH_AUDIO_BYTES / (1024 * 1024)
                )));
            }
            Ok((bytes, mime_type, filename))
        }
        AudioPayload::File { path, mime_type } => {
            let meta = tokio::fs::metadata(&path).await?;
            if meta.len() > MAX_BATCH_AUDIO_BYTES as u64 {
                return Err(SttError::UnsupportedAudio(format!(
                    "Audio file {} ({} bytes) exceeds {} MiB batch limit",
                    path.display(),
                    meta.len(),
                    MAX_BATCH_AUDIO_BYTES / (1024 * 1024)
                )));
            }
            let bytes = tokio::fs::read(&path).await?;
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("audio.bin")
                .to_string();
            Ok((bytes, mime_type, filename))
        }
    }
}

/// Map a `reqwest::Error` to the right `SttError` variant — timeouts and
/// connect failures are surfaced as `Network` so failover treats them as
/// retriable.
pub(super) fn classify_reqwest_error(e: &reqwest::Error) -> SttError {
    if e.is_timeout() {
        SttError::Network(format!("Request timed out: {e}"))
    } else if e.is_connect() {
        SttError::Network(format!("Connect failed: {e}"))
    } else {
        SttError::Network(e.to_string())
    }
}

/// Map an HTTP response status to a stable `SttError` variant. The body
/// snippet is truncated to 256 chars to keep keys/payloads out of logs.
pub(super) fn classify_http_status(status: reqwest::StatusCode, body: &str) -> SttError {
    let snippet = body.chars().take(256).collect::<String>();
    match status.as_u16() {
        401 | 403 => SttError::Auth(format!("HTTP {status}: {snippet}")),
        413 => SttError::UnsupportedAudio(format!("HTTP {status}: payload too large")),
        415 => SttError::UnsupportedAudio(format!("HTTP {status}: unsupported codec")),
        429 => SttError::RateLimit(format!("HTTP {status}: {snippet}")),
        500..=599 => SttError::ProviderUnavailable(format!("HTTP {status}: {snippet}")),
        _ => SttError::Other(format!("HTTP {status}: {snippet}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_http_status_maps_to_error_kinds() {
        assert_eq!(
            classify_http_status(reqwest::StatusCode::UNAUTHORIZED, "bad key").code(),
            "auth"
        );
        assert_eq!(
            classify_http_status(reqwest::StatusCode::PAYLOAD_TOO_LARGE, "").code(),
            "unsupported_audio"
        );
        assert_eq!(
            classify_http_status(reqwest::StatusCode::TOO_MANY_REQUESTS, "").code(),
            "rate_limit"
        );
        assert_eq!(
            classify_http_status(reqwest::StatusCode::BAD_GATEWAY, "").code(),
            "provider_unavailable"
        );
    }

    #[test]
    fn classify_http_status_truncates_long_bodies() {
        let body = "a".repeat(2_000);
        let err =
            classify_http_status(reqwest::StatusCode::INTERNAL_SERVER_ERROR, &body).to_string();
        assert!(err.len() < 400);
    }
}
