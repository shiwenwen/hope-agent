//! Azure Speech Service realtime WebSocket transcription.
//!
//! Reference docs (verified against the open-source SDK source):
//! - <https://learn.microsoft.com/azure/ai-services/speech-service/speech-services-quickstart-realtime-transcription>
//! - <https://github.com/microsoft/cognitive-services-speech-sdk-js/blob/master/src/common.speech/WebsocketMessageFormatter.ts>
//!   (USP frame encoder — ground truth for binary layout)
//! - <https://github.com/microsoft/cognitive-services-speech-sdk-js/blob/master/src/common.speech/ServiceRecognizerBase.ts>
//!   (turn-start sequence — speech.config → speech.context → WAV header → PCM)
//!
//! Endpoint: `wss://{region}.stt.speech.microsoft.com/speech/recognition/conversation/cognitiveservices/v1`
//! Auth header: `Ocp-Apim-Subscription-Key: <subscription_key>` (we skip
//! the STS token exchange to keep the dial-up cost to a single round-trip).
//!
//! USP wire shape (Universal Speech Protocol):
//! - Text frame: ASCII `Name: Value\r\n` headers + `\r\n` blank line +
//!   JSON body, all as a single WebSocket Text message.
//! - Binary frame: `[2-byte BE header_length][headers ASCII][body]`.
//!   `header_length` covers only the headers; body length is implied by
//!   the WebSocket message size. Headers terminate each line with `\r\n`
//!   but there's NO blank line separator (the length byte does that job).
//! - Required headers: `Path`, `X-RequestId` (sticky for the whole turn),
//!   `X-Timestamp`, `Content-Type`.
//!
//! Turn opening sequence (must be exactly this order — server tracks
//! state and rejects audio sent before the wave header):
//! 1. Text `Path: speech.config` — JSON device descriptor (one-time).
//! 2. Text `Path: speech.context` — JSON recognition config (per turn).
//! 3. Binary `Path: audio` `Content-Type: audio/x-wav` — 44-byte RIFF
//!    WAV header advertising the PCM format. **Without this** Azure
//!    fails to parse the stream because it expects `audio/x-wav` to be
//!    a real WAV container, not raw PCM.
//! 4. Binary `Path: audio` — PCM data chunks (~100 ms each).
//! 5. Binary `Path: audio` empty body — EOS sentinel.
//!
//! Server text frames return as the same HTTP-style envelope. We care
//! about `Path: speech.hypothesis` (partial) and `Path: speech.phrase`
//! (final, with `RecognitionStatus: "Success"`); other paths (turn.start,
//! turn.end, speech.startDetected, speech.endDetected) are advisory.
//!
//! Region resolution: Azure routes by hostname, so the base URL must
//! resolve to `wss://{region}.stt.speech.microsoft.com`. Two ways to get
//! there:
//! - Fill `extra.region` (e.g. `eastus`); we synthesise the URL.
//! - Or paste the full `wss://…stt.speech.microsoft.com` into `base_url`.
//! `extra.region` wins when both are set so the GUI hint is non-decorative.

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
use uuid::Uuid;

use crate::provider::AuthProfile;
use crate::stt::errors::{SttError, SttResult};
use crate::stt::types::{SttModelConfig, SttProviderConfig, TranscriptDelta, TranscriptOptions};

pub async fn open_stream(
    provider: &SttProviderConfig,
    _model: &SttModelConfig,
    profile: &AuthProfile,
    options: &TranscriptOptions,
) -> SttResult<super::SttStream> {
    let base_owned;
    let base = match provider
        .extra
        .get("region")
        .map(String::as_str)
        .filter(|s| !s.is_empty())
    {
        Some(region) => {
            base_owned = format!("wss://{}.stt.speech.microsoft.com", region);
            base_owned.as_str()
        }
        None => provider.resolve_base_url(profile).trim_end_matches('/'),
    };
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

    let https_twin = super::ws_to_https_twin(&url, "Azure Speech")?;
    provider.check_ssrf(&https_twin).await?;

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

    let ws = super::ws_connect_with_caps(request, "Azure Speech").await?;
    let (mut ws_sink, mut ws_stream) = ws.split();

    // Opening sequence per ServiceRecognizerBase.ts — Azure expects
    // speech.config (one-time device descriptor), speech.context (per
    // turn recognition config), and a WAV RIFF header binary frame
    // BEFORE any PCM audio. Missing the RIFF header is silently swallowed
    // by the server because `Content-Type: audio/x-wav` advertises a
    // WAV container.
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

    // speech.context — declare conversation/dictation mode and ask for
    // detailed output (matches `?format=detailed` query). SDK builds
    // this dynamically with phrase lists / pronunciation scoring; the
    // minimal form here is sufficient for plain ASR.
    let ctx_body = json!({
        "phraseDetection": { "mode": "Interactive" },
        "phraseOutput": {
            "format": "Detailed",
            "detailed": { "options": [] }
        }
    });
    let ctx_frame = build_text_frame("speech.context", &request_id, &ctx_body.to_string());
    ws_sink
        .send(Message::Text(ctx_frame.into()))
        .await
        .map_err(|e| SttError::Network(format!("Azure speech.context send: {e}")))?;

    let sample_rate = options.sample_rate_hz.unwrap_or(16_000);
    let wav_header = build_wav_header(sample_rate, 1, 16);
    let wav_frame = build_binary_frame("audio", &request_id, Some("audio/x-wav"), &wav_header);
    ws_sink
        .send(Message::Binary(wav_frame.into()))
        .await
        .map_err(|e| SttError::Network(format!("Azure WAV header send: {e}")))?;

    let (audio_tx, mut audio_rx) = mpsc::channel::<Vec<u8>>(super::STT_STREAM_CHANNEL_CAPACITY);
    let (delta_tx, delta_rx) =
        mpsc::channel::<Result<TranscriptDelta, SttError>>(super::STT_STREAM_CHANNEL_CAPACITY);

    let request_id_send = request_id.clone();
    tokio::spawn(async move {
        while let Some(chunk) = audio_rx.recv().await {
            let frame = build_binary_frame("audio", &request_id_send, None, &chunk);
            if ws_sink.send(Message::Binary(frame.into())).await.is_err() {
                break;
            }
        }
        // EOS: empty-body audio frame. Server still flushes its final
        // speech.phrase + turn.end before sending its own Close; we
        // don't initiate Close here to avoid racing those.
        let eos = build_binary_frame("audio", &request_id_send, None, &[]);
        let _ = ws_sink.send(Message::Binary(eos.into())).await;
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
                        .send(Err(SttError::Network(format!("Azure Speech WS recv: {e}"))))
                        .await;
                    break;
                }
            }
        }
    });

    Ok(super::SttStream { audio_tx, delta_rx })
}

fn iso8601_now() -> String {
    // Azure accepts seconds precision; format matches the documented
    // X-Timestamp shape (e.g. "2024-01-15T12:34:56Z").
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
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

/// 44-byte canonical PCM WAV (RIFF) header for streaming audio. All
/// multi-byte fields are little-endian per the WAV spec; lengths are
/// left at 0 because the stream length is unknown up front (matches
/// the Microsoft SDK `AudioStreamFormatImpl` private header buffer).
fn build_wav_header(sample_rate: u32, channels: u16, bits_per_sample: u16) -> Vec<u8> {
    let byte_rate = sample_rate * u32::from(channels) * u32::from(bits_per_sample) / 8;
    let block_align = channels * (bits_per_sample / 8);
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&0u32.to_le_bytes()); // file length — 0 = unknown
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    h.extend_from_slice(&1u16.to_le_bytes()); // PCM format tag
    h.extend_from_slice(&channels.to_le_bytes());
    h.extend_from_slice(&sample_rate.to_le_bytes());
    h.extend_from_slice(&byte_rate.to_le_bytes());
    h.extend_from_slice(&block_align.to_le_bytes());
    h.extend_from_slice(&bits_per_sample.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&0u32.to_le_bytes()); // data length — 0 = unknown
    h
}

fn build_binary_frame(
    path: &str,
    request_id: &str,
    content_type: Option<&str>,
    body: &[u8],
) -> Vec<u8> {
    let mut headers = format!(
        "Path: {}\r\nX-RequestId: {}\r\nX-Timestamp: {}\r\n",
        path,
        request_id,
        iso8601_now()
    );
    // Per the open-source JS SDK (`ServiceRecognizerBase.ts:sendAudio`),
    // only the leading RIFF header carries `Content-Type: audio/x-wav`;
    // subsequent raw-PCM chunks and the EOS frame pass `null`, which
    // omits the header entirely. Repeating `audio/x-wav` on every chunk
    // makes strict servers treat each chunk as a fresh WAV container.
    if let Some(ct) = content_type {
        headers.push_str("Content-Type: ");
        headers.push_str(ct);
        headers.push_str("\r\n");
    }
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
        start_ms: value
            .get("Offset")
            .and_then(|v| v.as_u64())
            .map(|t| t / 10_000),
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
        let f = build_binary_frame("audio", "rid-1", Some("audio/x-wav"), body);
        let header_len = u16::from_be_bytes([f[0], f[1]]) as usize;
        // First 2 bytes are length; next `header_len` bytes are ASCII headers.
        let header_bytes = &f[2..2 + header_len];
        let s = std::str::from_utf8(header_bytes).unwrap();
        assert!(s.starts_with("Path: audio\r\n"));
        assert!(s.contains("Content-Type: audio/x-wav\r\n"));
        assert_eq!(&f[2 + header_len..], body);
    }

    #[test]
    fn build_binary_frame_omits_content_type_when_none() {
        // Mirrors the JS SDK: only the RIFF header carries
        // `audio/x-wav`; subsequent PCM chunks pass `null`, which
        // omits the header entirely.
        let f = build_binary_frame("audio", "rid-2", None, b"PCM");
        let header_len = u16::from_be_bytes([f[0], f[1]]) as usize;
        let header_bytes = std::str::from_utf8(&f[2..2 + header_len]).unwrap();
        assert!(header_bytes.starts_with("Path: audio\r\n"));
        assert!(
            !header_bytes.contains("Content-Type"),
            "PCM chunks must NOT advertise a content type"
        );
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
    fn build_wav_header_matches_canonical_pcm16_16khz_mono_layout() {
        let h = build_wav_header(16_000, 1, 16);
        assert_eq!(h.len(), 44);
        assert_eq!(&h[0..4], b"RIFF");
        assert_eq!(&h[8..16], b"WAVEfmt ");
        // fmt chunk size = 16 (LE u32)
        assert_eq!(u32::from_le_bytes([h[16], h[17], h[18], h[19]]), 16);
        // format tag = 1 (PCM)
        assert_eq!(u16::from_le_bytes([h[20], h[21]]), 1);
        // channels = 1
        assert_eq!(u16::from_le_bytes([h[22], h[23]]), 1);
        // sample rate = 16000
        assert_eq!(u32::from_le_bytes([h[24], h[25], h[26], h[27]]), 16_000);
        // byte rate = 16000 * 1 * 2 = 32000
        assert_eq!(u32::from_le_bytes([h[28], h[29], h[30], h[31]]), 32_000);
        // block align = 1 * 2
        assert_eq!(u16::from_le_bytes([h[32], h[33]]), 2);
        // bits per sample = 16
        assert_eq!(u16::from_le_bytes([h[34], h[35]]), 16);
        assert_eq!(&h[36..40], b"data");
    }

    #[test]
    fn iso8601_emits_basic_z_format_with_current_time() {
        let s = iso8601_now();
        // YYYY-MM-DDTHH:MM:SSZ — exactly 20 chars, suffix Z.
        assert_eq!(s.len(), 20);
        assert!(s.ends_with('Z'));
        assert_eq!(s.as_bytes()[10], b'T');
    }
}
