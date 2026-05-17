//! Azure Speech Service realtime WebSocket transcription.
//!
//! `wss://{region}.stt.speech.microsoft.com/speech/recognition/conversation/cognitiveservices/v1`
//! Auth header: `Ocp-Apim-Subscription-Key: <subscription_key>` (we skip
//! the optional STS token exchange to keep the dial-up cost to a single
//! round-trip).
//!
//! Wire shape (Azure's "USP" / Universal Speech Protocol):
//! - Each frame is HTTP-like: ASCII headers terminated by `\r\n\r\n`,
//!   then JSON body (text frames) or raw audio bytes (binary frames).
//! - Required headers: `Path`, `X-RequestId` (sticky for the whole turn),
//!   `X-Timestamp`, `Content-Type`.
//! - Client opening frame: `Path: speech.config` carrying a JSON device
//!   descriptor. Subsequent audio frames are binary with a 2-byte BE
//!   header-length prefix followed by the header block then PCM bytes.
//! - Server text frames return as the same HTTP-style envelope. We care
//!   about `Path: speech.hypothesis` (partial) and `Path: speech.phrase`
//!   (final, with `RecognitionStatus: "Success"`).
//!
//! `region` belongs in `provider.extra` (e.g. `extra.region="eastus"`)
//! when the user pastes only the subscription key; otherwise the base
//! URL must already include the region subdomain.

use std::time::SystemTime;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
use uuid::Uuid;

use crate::provider::AuthProfile;
use crate::security::ssrf::{check_url, SsrfPolicy};
use crate::stt::errors::{SttError, SttResult};
use crate::stt::types::{
    SttModelConfig, SttProviderConfig, TranscriptDelta, TranscriptOptions,
};

const MAX_WS_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_WS_FRAME_BYTES: usize = 1024 * 1024;

pub async fn open_stream(
    provider: &SttProviderConfig,
    _model: &SttModelConfig,
    profile: &AuthProfile,
    options: &TranscriptOptions,
) -> SttResult<super::SttStream> {
    let base = provider.resolve_base_url(profile).trim_end_matches('/');
    let mut url = format!(
        "{}/speech/recognition/conversation/cognitiveservices/v1",
        base
    );
    if let Some(lang) = options.language.as_deref().filter(|l| !l.is_empty()) {
        url.push_str(&format!("?language={}", urlencoding::encode(lang)));
        url.push_str("&format=detailed");
    } else {
        url.push_str("?format=detailed");
    }

    let https_twin = ws_to_https_twin(&url)?;
    let cfg = crate::config::cached_config();
    let policy = if provider.allow_private_network {
        SsrfPolicy::AllowPrivate
    } else {
        cfg.ssrf.default_policy
    };
    check_url(&https_twin, policy, &cfg.ssrf.trusted_hosts)
        .await
        .map_err(|e| SttError::SsrfBlocked(e.to_string()))?;

    let request_id = Uuid::new_v4().simple().to_string();
    let connection_id = Uuid::new_v4().simple().to_string();

    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|e| SttError::Other(format!("Invalid Azure Speech URL: {e}")))?;
    request.headers_mut().insert(
        "Ocp-Apim-Subscription-Key",
        profile
            .api_key
            .parse()
            .map_err(|e| SttError::Other(format!("Bad Azure subscription key: {e}")))?,
    );
    request.headers_mut().insert(
        "X-ConnectionId",
        connection_id
            .parse()
            .map_err(|e| SttError::Other(format!("Bad connection id: {e}")))?,
    );

    let ws_config = WebSocketConfig::default()
        .max_message_size(Some(MAX_WS_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_WS_FRAME_BYTES));
    let (ws, _resp) = tokio_tungstenite::connect_async_with_config(request, Some(ws_config), false)
        .await
        .map_err(|e| SttError::Network(format!("Azure Speech WS connect failed: {e}")))?;
    let (mut ws_sink, mut ws_stream) = ws.split();

    // Opening handshake — Azure requires a `speech.config` text frame
    // before any audio. The device descriptor is mostly cosmetic but
    // must be syntactically valid.
    let cfg_body = json!({
        "context": {
            "system": { "name": "hope-agent", "version": env!("CARGO_PKG_VERSION") },
            "os": { "platform": std::env::consts::OS, "name": std::env::consts::FAMILY, "version": "1" },
            "device": { "manufacturer": "hope-agent", "model": "stt", "version": "1" }
        }
    });
    let config_frame = build_text_frame("speech.config", &request_id, &cfg_body.to_string());
    ws_sink
        .send(Message::Text(config_frame.into()))
        .await
        .map_err(|e| SttError::Network(format!("Azure speech.config send: {e}")))?;

    let (audio_tx, mut audio_rx) = mpsc::channel::<Vec<u8>>(64);
    let (delta_tx, delta_rx) = mpsc::channel::<Result<TranscriptDelta, SttError>>(64);

    let request_id_send = request_id.clone();
    tokio::spawn(async move {
        while let Some(chunk) = audio_rx.recv().await {
            let frame = build_binary_frame("audio", &request_id_send, &chunk);
            if ws_sink.send(Message::Binary(frame.into())).await.is_err() {
                break;
            }
        }
        // Send the empty-body audio frame Azure treats as EOS.
        let eos = build_binary_frame("audio", &request_id_send, &[]);
        let _ = ws_sink.send(Message::Binary(eos.into())).await;
        let _ = ws_sink.send(Message::Close(None)).await;
    });

    let session_id = String::new();
    let request_id_recv = request_id;
    tokio::spawn(async move {
        while let Some(msg) = ws_stream.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Some(delta) = parse_text_frame(&session_id, &request_id_recv, &text) {
                        if delta_tx.send(Ok(delta)).await.is_err() {
                            break;
                        }
                    }
                }
                Ok(Message::Binary(_))
                | Ok(Message::Frame(_))
                | Ok(Message::Ping(_))
                | Ok(Message::Pong(_)) => {}
                Ok(Message::Close(_)) => break,
                Err(e) => {
                    let _ = delta_tx
                        .send(Err(SttError::Network(format!(
                            "Azure Speech WS recv: {e}"
                        ))))
                        .await;
                    break;
                }
            }
        }
    });

    Ok(super::SttStream { audio_tx, delta_rx })
}

fn iso8601_now() -> String {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Minimal RFC3339 / ISO8601 — Azure accepts seconds precision.
    let datetime = chrono_like_iso(secs);
    datetime
}

/// Tiny ISO-8601 formatter (`YYYY-MM-DDTHH:MM:SSZ`) so we don't pull
/// chrono just for one header field. Computes via `time` math.
fn chrono_like_iso(secs: i64) -> String {
    // Days / time-of-day.
    let day = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let hh = tod / 3600;
    let mm = (tod / 60) % 60;
    let ss = tod % 60;
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = day + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y_civil = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y_civil, m, d, hh, mm, ss)
}

fn build_text_frame(path: &str, request_id: &str, body: &str) -> String {
    let headers = format!(
        "Path: {}\r\nX-RequestId: {}\r\nX-Timestamp: {}\r\nContent-Type: application/json; charset=utf-8\r\n\r\n",
        path,
        request_id,
        iso8601_now()
    );
    format!("{}{}", headers, body)
}

fn build_binary_frame(path: &str, request_id: &str, body: &[u8]) -> Vec<u8> {
    let headers = format!(
        "Path: {}\r\nX-RequestId: {}\r\nX-Timestamp: {}\r\nContent-Type: audio/x-wav\r\n",
        path,
        request_id,
        iso8601_now()
    );
    let header_bytes = headers.as_bytes();
    let header_len = header_bytes.len() as u16;
    let mut frame = Vec::with_capacity(2 + header_bytes.len() + body.len());
    frame.extend_from_slice(&header_len.to_be_bytes());
    frame.extend_from_slice(header_bytes);
    frame.extend_from_slice(body);
    frame
}

/// Split an Azure text frame into `(headers map, body str)`. Headers are
/// `\r\n` separated; an empty line marks the body boundary.
fn split_text_frame(raw: &str) -> Option<(std::collections::HashMap<String, String>, &str)> {
    let mut headers = std::collections::HashMap::new();
    let (head, body) = raw.split_once("\r\n\r\n")?;
    for line in head.split("\r\n") {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    Some((headers, body))
}

fn parse_text_frame(
    session_id: &str,
    expected_request_id: &str,
    raw: &str,
) -> Option<TranscriptDelta> {
    let (headers, body) = split_text_frame(raw)?;
    let path = headers.get("path")?;
    if let Some(rid) = headers.get("x-requestid") {
        if rid != expected_request_id {
            return None;
        }
    }
    let value: Value = serde_json::from_str(body).ok()?;
    let (text, is_final) = match path.as_str() {
        "speech.hypothesis" => (value.get("Text").and_then(|v| v.as_str())?, false),
        "speech.phrase" => {
            let status = value
                .get("RecognitionStatus")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if status != "Success" {
                return None;
            }
            // `detailed` mode returns NBest[0].Display when format=detailed;
            // simple mode returns DisplayText. Try both.
            let text = value
                .get("DisplayText")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    value
                        .get("NBest")
                        .and_then(|n| n.get(0))
                        .and_then(|n| n.get("Display"))
                        .and_then(|v| v.as_str())
                })?;
            (text, true)
        }
        _ => return None,
    };
    if text.is_empty() {
        return None;
    }
    Some(TranscriptDelta {
        session_id: session_id.to_string(),
        text: text.to_string(),
        is_final,
        start_ms: value.get("Offset").and_then(|v| v.as_u64()).map(|t| t / 10_000),
        end_ms: value
            .get("Offset")
            .and_then(|v| v.as_u64())
            .zip(value.get("Duration").and_then(|v| v.as_u64()))
            .map(|(o, d)| (o + d) / 10_000),
        confidence: None,
        language: None,
        accumulated: None,
    })
}

fn ws_to_https_twin(url: &str) -> SttResult<String> {
    let mut parsed = url::Url::parse(url)
        .map_err(|e| SttError::Other(format!("Invalid Azure Speech URL: {e}")))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_text_frame_includes_required_headers() {
        let f = build_text_frame("speech.config", "abc", r#"{"context":{}}"#);
        assert!(f.contains("Path: speech.config\r\n"));
        assert!(f.contains("X-RequestId: abc\r\n"));
        assert!(f.contains("X-Timestamp: "));
        assert!(f.contains("Content-Type: application/json"));
        assert!(f.ends_with(r#"{"context":{}}"#));
    }

    #[test]
    fn build_binary_frame_prefixes_header_length() {
        let body = b"PCMDATA";
        let f = build_binary_frame("audio", "rid-1", body);
        let header_len = u16::from_be_bytes([f[0], f[1]]) as usize;
        // First 2 bytes are length; next `header_len` bytes are ASCII headers.
        let header_bytes = &f[2..2 + header_len];
        let s = std::str::from_utf8(header_bytes).unwrap();
        assert!(s.starts_with("Path: audio\r\n"));
        assert_eq!(&f[2 + header_len..], body);
    }

    #[test]
    fn parse_hypothesis_returns_partial() {
        let raw = "Path: speech.hypothesis\r\nX-RequestId: rid\r\nContent-Type: application/json\r\n\r\n{\"Text\":\"hello world\",\"Offset\":100000,\"Duration\":50000}";
        let d = parse_text_frame("s", "rid", raw).expect("hypothesis should parse");
        assert_eq!(d.text, "hello world");
        assert!(!d.is_final);
        assert_eq!(d.start_ms, Some(10));
        assert_eq!(d.end_ms, Some(15));
    }

    #[test]
    fn parse_phrase_success_returns_final() {
        let raw = "Path: speech.phrase\r\nX-RequestId: rid\r\nContent-Type: application/json\r\n\r\n{\"RecognitionStatus\":\"Success\",\"DisplayText\":\"the quick fox\",\"Offset\":0,\"Duration\":1000000}";
        let d = parse_text_frame("s", "rid", raw).expect("phrase should parse");
        assert_eq!(d.text, "the quick fox");
        assert!(d.is_final);
    }

    #[test]
    fn parse_phrase_non_success_skipped() {
        let raw = "Path: speech.phrase\r\nX-RequestId: rid\r\nContent-Type: application/json\r\n\r\n{\"RecognitionStatus\":\"NoMatch\"}";
        assert!(parse_text_frame("s", "rid", raw).is_none());
    }

    #[test]
    fn parse_rejects_mismatched_request_id() {
        let raw = "Path: speech.hypothesis\r\nX-RequestId: other\r\n\r\n{\"Text\":\"ignored\"}";
        assert!(parse_text_frame("s", "rid", raw).is_none());
    }

    #[test]
    fn iso8601_emits_basic_z_format() {
        // 1970-01-01T00:00:00Z corresponds to epoch 0
        assert_eq!(chrono_like_iso(0), "1970-01-01T00:00:00Z");
        // 2024-01-15T12:34:56Z = 1705322096
        assert_eq!(chrono_like_iso(1705322096), "2024-01-15T12:34:56Z");
    }
}
