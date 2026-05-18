//! Volcengine (火山引擎 / 字节豆包) streaming ASR WebSocket — BigModel
//! generation (`/api/v3/sauc/bigmodel`).
//!
//! Reference docs:
//! - <https://www.volcengine.com/docs/6561/1354869> (大模型流式语音识别 API)
//! - <https://www.volcengine.com/docs/6561/1096680> (豆包语音 API 接口文档)
//!
//! WS upgrade headers — exactly four, matching the official Python demo
//! shipped from the docs page (`sauc_python.zip`):
//! - `X-Api-Resource-Id: <resource>` — defaults to the 1.0 hourly
//!   resource `volc.bigasr.sauc.duration`. For the 2.0 "Seed" tier
//!   (instances whose id contains `Speech_Recognition_Seed_streaming…`),
//!   override `extra.resource_id` to `volc.seedasr.sauc.duration`.
//! - `X-Api-Request-Id: <UUID>` — hyphenated UUID; echoed back as
//!   `X-Tt-Logid` for debugging.
//! - `X-Api-Access-Key: <access_key>` — `provider.api_key` (the
//!   "Access Token" — NOT the IAM Secret Key).
//! - `X-Api-App-Key: <app_key>` — `extra.app_key` (the "APP ID" digit
//!   string in the Volcengine console).
//!
//! The docs table mentions `X-Api-Sequence: -1` and `X-Api-Connect-Id`
//! but the official Python demo sends NEITHER; the doc table is
//! describing the wire protocol's body-level sequence field, not an
//! HTTP header. Adding either to the WS upgrade is rejected with 400.
//!
//! Body framing (all frames, matches `RequestBuilder.new_*_request` in
//! the official demo):
//! ```text
//! [4-byte header][4-byte signed sequence][4-byte unsigned payload_size][gzip(payload)]
//! ```
//! ALL frames (including the first config frame) carry a signed i32
//! sequence number and `flags=POS_SEQUENCE` (0b0001). The terminal audio
//! frame sets `flags=NEG_WITH_SEQUENCE` (0b0011) AND negates the
//! sequence value (`seq = -seq`); the flag bit alone is not enough.
//!
//! Binary framing (BigModel protocol, all frames):
//! ```text
//! byte 0: (version<<4) | header_size_in_4byte_units   // 0x11
//! byte 1: (msg_type<<4) | flags                       // see below
//! byte 2: (serialization<<4) | compression            // 0x11 = JSON/gzip,
//!                                                     // 0xf1 = raw/gzip
//! byte 3: reserved (0)
//! bytes 4..8:  payload size, u32 big-endian
//! bytes 8..  : gzip(payload)
//! ```
//! Message types (per ByteDance BigModel WS docs): 0x1 client config,
//! 0x2 client audio, 0x9 (=0b1001) full server response, 0xf (=0b1111)
//! server error. Audio flags: 0x1 = positive seq (continue + sequence
//! number embedded), 0x2 = negative seq (last frame).
//!
//! Server JSON shape after gunzip:
//! `{"result":{"text":"...","utterances":[{"text":"...","definite":bool,
//!   "start_time":ms,"end_time":ms}]}}`
//! `definite=true` ⇒ final utterance; partial deltas come as multiple
//! `definite=false` frames sharing prefix text.
//!
//! Sequence handling: when `flags & 0x1` is set ("positive sequence")
//! the frame carries a 4-byte BE sequence number BEFORE the payload size
//! prefix. Audio client frames always set this flag and increment the
//! sequence per chunk; server full-response frames carry the matching
//! reply sequence. Frames without the flag have no sequence field.

use std::io::Read;

use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
use uuid::Uuid;

use crate::provider::AuthProfile;
use crate::stt::errors::{SttError, SttResult};
use crate::stt::types::{SttModelConfig, SttProviderConfig, TranscriptDelta, TranscriptOptions};

const DEFAULT_RESOURCE_ID: &str = "volc.bigasr.sauc.duration";

const MSG_CLIENT_CONFIG: u8 = 0x1;
const MSG_CLIENT_AUDIO: u8 = 0x2;
/// Full server response — per ByteDance BigModel WS contract, type 0b1001.
const MSG_SERVER_RESPONSE: u8 = 0x9;
/// Server error frame — per the same contract, type 0b1111.
const MSG_SERVER_ERROR: u8 = 0xf;

/// `flags & FLAG_SEQUENCE` (bit 0): 4-byte signed BE sequence number is
/// embedded in the frame between the fixed header and the payload size.
const FLAG_SEQUENCE: u8 = 0x1;
/// `flags & FLAG_NEGATIVE` (bit 1): terminal frame marker. Client must
/// pair with FLAG_SEQUENCE and a negated sequence value (per the
/// official Python demo).
const FLAG_NEGATIVE: u8 = 0x2;
/// `flags & FLAG_EVENT` (bit 2): server-only — a 4-byte event ID
/// follows the sequence field (if present) before the payload size.
/// Required to parse server ack frames correctly.
const FLAG_EVENT: u8 = 0x4;

const SER_JSON: u8 = 0x1;
const SER_NONE: u8 = 0x0;
const COMPRESS_NONE: u8 = 0x0;
const COMPRESS_GZIP: u8 = 0x1;

pub async fn open_stream(
    provider: &SttProviderConfig,
    model: &SttModelConfig,
    profile: &AuthProfile,
    options: &TranscriptOptions,
) -> SttResult<super::SttStream> {
    let app_key = provider.require_extra("app_key", "AppKey")?.to_string();
    let resource_id = provider
        .extra
        .get("resource_id")
        .map(String::as_str)
        .unwrap_or(DEFAULT_RESOURCE_ID)
        .to_string();

    let base = provider.resolve_base_url(profile).trim_end_matches('/');
    let url = format!("{}/api/v3/sauc/bigmodel", base);

    let https_twin = super::ws_to_https_twin(&url, "Volcengine")?;
    provider.check_ssrf(&https_twin).await?;

    // Volcengine wants the hyphenated UUID form (`xxxxxxxx-xxxx-xxxx-…`),
    // not the compact 32-hex `simple()` form — the latter is silently
    // rejected as a malformed Connect-Id → 400 Bad Request on upgrade.
    let request_id = Uuid::new_v4().to_string();

    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|e| SttError::Other(format!("Invalid Volcengine URL: {e}")))?;
    let headers = request.headers_mut();
    headers.insert(
        "X-Api-App-Key",
        app_key
            .parse()
            .map_err(|e| SttError::Other(format!("Bad X-Api-App-Key value: {e}")))?,
    );
    headers.insert(
        "X-Api-Access-Key",
        profile
            .api_key
            .parse()
            .map_err(|e| SttError::Other(format!("Bad X-Api-Access-Key value: {e}")))?,
    );
    headers.insert(
        "X-Api-Resource-Id",
        resource_id
            .parse()
            .map_err(|e| SttError::Other(format!("Bad X-Api-Resource-Id value: {e}")))?,
    );
    headers.insert(
        "X-Api-Request-Id",
        request_id
            .parse()
            .map_err(|e| SttError::Other(format!("Bad X-Api-Request-Id value: {e}")))?,
    );

    // Pre-connect diagnostic: ResourceId mismatch (BigASR 1.0 vs Seed 2.0)
    // is the #1 cause of an opaque 400 here. Logging the resolved values
    // (sans keys) makes the failure self-diagnosable from the log file.
    crate::app_info!(
        "stt",
        "volcengine::open_stream",
        "WS connect url={} resource_id={} model.id={} model_name=bigmodel app_key_len={} access_key_len={} request_id={}",
        url,
        resource_id,
        model.id,
        app_key.len(),
        profile.api_key.len(),
        request_id
    );

    let ws = super::ws_connect_with_caps(request, "Volcengine").await?;
    let (mut ws_sink, mut ws_stream) = ws.split();

    let language = options
        .language
        .as_deref()
        .filter(|l| !l.is_empty())
        .unwrap_or("zh-CN")
        .to_string();
    let sample_rate = options.sample_rate_hz.unwrap_or(16_000);
    // Body shape per the docs «发送 full client request» section:
    // - `audio.rate` (NOT sample_rate), `audio.language` (NOT under request).
    // - `request.model_name` is the wire model name. The docs state
    //   "目前只有 bigmodel"; the user-configured `model.id` is the
    //   console instance display name (e.g. Speech_Recognition_Big_…)
    //   and is rejected by the BigModel runtime, so we hardcode the
    //   protocol model_name here. Tier selection (1.0 vs 2.0) flows
    //   through `X-Api-Resource-Id` above, not this field.
    let cfg_body = json!({
        "user": { "uid": request_id },
        "audio": {
            "format": "pcm",
            "codec": "raw",
            "rate": sample_rate,
            "bits": 16,
            "channel": 1,
            "language": language,
        },
        "request": {
            "model_name": "bigmodel",
            "enable_itn": true,
            "enable_punc": true,
            "show_utterances": true,
        }
    });
    // Config frame is seq=1 per the official demo. All client frames —
    // including config — carry a signed sequence number and the
    // POS_SEQUENCE flag (0b0001).
    let config_frame = build_frame(
        MSG_CLIENT_CONFIG,
        FLAG_SEQUENCE,
        Some(1),
        SER_JSON,
        COMPRESS_GZIP,
        &cfg_body.to_string().into_bytes(),
    )?;
    ws_sink
        .send(Message::Binary(config_frame.into()))
        .await
        .map_err(|e| SttError::Network(format!("Volcengine config send: {e}")))?;

    let (audio_tx, mut audio_rx) = mpsc::channel::<Vec<u8>>(super::STT_STREAM_CHANNEL_CAPACITY);
    let (delta_tx, delta_rx) =
        mpsc::channel::<Result<TranscriptDelta, SttError>>(super::STT_STREAM_CHANNEL_CAPACITY);

    tokio::spawn(async move {
        // Audio frames MUST be gzipped per the official demo; config
        // frame consumed seq=1 so audio starts at seq=2.
        let mut seq: i32 = 2;
        let mut pending: Option<Vec<u8>> = None;
        loop {
            let chunk = match audio_rx.recv().await {
                Some(c) => c,
                None => break,
            };
            if let Some(prev) = pending.take() {
                match build_frame(
                    MSG_CLIENT_AUDIO,
                    FLAG_SEQUENCE,
                    Some(seq),
                    SER_NONE,
                    COMPRESS_GZIP,
                    &prev,
                ) {
                    Ok(frame) => {
                        if ws_sink.send(Message::Binary(frame.into())).await.is_err() {
                            return;
                        }
                        seq = seq.wrapping_add(1);
                    }
                    Err(_) => return,
                }
            }
            pending = Some(chunk);
        }
        // Terminal audio frame: NEG_WITH_SEQUENCE flag + negated seq.
        // If the user clicked stop before any chunk arrived, send an
        // empty terminal frame so the server still observes EOS.
        let last_body = pending.unwrap_or_default();
        if let Ok(frame) = build_frame(
            MSG_CLIENT_AUDIO,
            FLAG_SEQUENCE | FLAG_NEGATIVE,
            Some(-seq),
            SER_NONE,
            COMPRESS_GZIP,
            &last_body,
        ) {
            let _ = ws_sink.send(Message::Binary(frame.into())).await;
        }
        // Don't send a WebSocket Close here — server may still be
        // delivering the final transcript after EOS. Dropping ws_sink at
        // task end + server's own Close frame handle the lifecycle;
        // session::finalize's 30s timeout backstops a stuck server.
    });

    let session_id = String::new();
    tokio::spawn(async move {
        while let Some(msg) = ws_stream.next().await {
            match msg {
                Ok(Message::Binary(bytes)) => match parse_frame(&bytes) {
                    Ok(payload) => {
                        if let Some(delta) = parse_response(&session_id, &payload) {
                            if delta_tx.send(Ok(delta)).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        let _ = delta_tx.send(Err(e)).await;
                        break;
                    }
                },
                Ok(Message::Text(_))
                | Ok(Message::Frame(_))
                | Ok(Message::Ping(_))
                | Ok(Message::Pong(_)) => {}
                Ok(Message::Close(_)) => break,
                Err(e) => {
                    let _ = delta_tx
                        .send(Err(SttError::Network(format!("Volcengine WS recv: {e}"))))
                        .await;
                    break;
                }
            }
        }
    });

    Ok(super::SttStream { audio_tx, delta_rx })
}

fn build_frame(
    msg_type: u8,
    flags: u8,
    sequence: Option<i32>,
    serialization: u8,
    compression: u8,
    payload: &[u8],
) -> SttResult<Vec<u8>> {
    // Sanity: if a sequence is supplied, the flag bit must be set so the
    // receiver knows to read it. Programmer error if not.
    debug_assert!(
        sequence.is_none() || (flags & FLAG_SEQUENCE) != 0,
        "FLAG_SEQUENCE must be set when sending a sequence number"
    );

    let body = if compression == COMPRESS_GZIP {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        std::io::Write::write_all(&mut encoder, payload)
            .map_err(|e| SttError::Other(format!("Volcengine gzip encode: {e}")))?;
        encoder
            .finish()
            .map_err(|e| SttError::Other(format!("Volcengine gzip finalize: {e}")))?
    } else {
        payload.to_vec()
    };

    let capacity = 8 + sequence.map(|_| 4).unwrap_or(0) + body.len();
    let mut frame = Vec::with_capacity(capacity);
    // protocol_version=1, header_size=1 (4 bytes total header).
    frame.push((1 << 4) | 1);
    frame.push((msg_type << 4) | (flags & 0x0f));
    frame.push((serialization << 4) | (compression & 0x0f));
    frame.push(0); // reserved
    if let Some(seq) = sequence {
        // Signed BE: terminal audio frame uses negated sequence per the
        // official Python demo (`struct.pack('>i', -seq)`).
        frame.extend_from_slice(&seq.to_be_bytes());
    }
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedFrame {
    msg_type: u8,
    flags: u8,
    serialization: u8,
    sequence: Option<i32>,
    body: Vec<u8>,
}

/// Server frame layout per `ResponseParser.parse_response` in the official
/// Python demo:
///
/// ```text
/// [4-byte header]
/// [4-byte signed sequence]    if flags & 0x01
/// (is_last marker, no bytes)  if flags & 0x02
/// [4-byte event id]           if flags & 0x04   ← Hope Agent missed this
/// [4-byte error_code]         if msg_type == SERVER_ERROR
/// [4-byte payload_size]
/// [gzip(payload)]
/// ```
///
/// The Python demo does NOT validate `payload_size` against the actual
/// remaining bytes — it just gunzips whatever's left. We mirror that:
/// trust the WebSocket message boundary, treat the size field as
/// informational, ignore the declared length entirely.
fn parse_frame(bytes: &[u8]) -> SttResult<ParsedFrame> {
    if bytes.len() < 4 {
        return Err(SttError::Other("Volcengine frame too short".into()));
    }
    let header_words = bytes[0] & 0x0f;
    let header_len = header_words as usize * 4;
    if header_len == 0 || bytes.len() < header_len {
        return Err(SttError::Other("Volcengine frame header invalid".into()));
    }
    let msg_type = (bytes[1] >> 4) & 0x0f;
    let flags = bytes[1] & 0x0f;
    let ser_compress = bytes[2];
    let serialization = (ser_compress >> 4) & 0x0f;
    let compression = ser_compress & 0x0f;

    let mut cursor = header_len;
    let sequence = if (flags & FLAG_SEQUENCE) != 0 {
        if bytes.len() < cursor + 4 {
            return Err(SttError::Other(
                "Volcengine frame missing sequence number".into(),
            ));
        }
        let seq = i32::from_be_bytes([
            bytes[cursor],
            bytes[cursor + 1],
            bytes[cursor + 2],
            bytes[cursor + 3],
        ]);
        cursor += 4;
        Some(seq)
    } else {
        None
    };

    if (flags & FLAG_EVENT) != 0 {
        // Skip 4-byte event ID — not used by Hope Agent today, but
        // server's first ack frame after the config request sets this.
        if bytes.len() < cursor + 4 {
            return Err(SttError::Other("Volcengine frame missing event id".into()));
        }
        cursor += 4;
    }

    // Error frames carry a 4-byte error code before payload_size.
    let error_code = if msg_type == MSG_SERVER_ERROR {
        if bytes.len() < cursor + 4 {
            return Err(SttError::Other(
                "Volcengine error frame missing error code".into(),
            ));
        }
        let code = i32::from_be_bytes([
            bytes[cursor],
            bytes[cursor + 1],
            bytes[cursor + 2],
            bytes[cursor + 3],
        ]);
        cursor += 4;
        Some(code)
    } else {
        None
    };

    if bytes.len() < cursor + 4 {
        return Err(SttError::Other(
            "Volcengine frame missing payload size prefix".into(),
        ));
    }
    // payload_size is documented as the gzip-compressed payload length.
    // We deliberately ignore it and use the WebSocket message boundary
    // instead (cursor+4 to end of buffer), matching the demo.
    cursor += 4;
    let raw_body = &bytes[cursor..];

    let body = match compression {
        COMPRESS_GZIP => {
            if raw_body.is_empty() {
                Vec::new()
            } else {
                let mut decoder = GzDecoder::new(raw_body);
                let mut out = Vec::new();
                decoder
                    .read_to_end(&mut out)
                    .map_err(|e| SttError::Other(format!("Volcengine gzip decode: {e}")))?;
                out
            }
        }
        COMPRESS_NONE => raw_body.to_vec(),
        other => {
            return Err(SttError::Other(format!(
                "Volcengine unsupported compression {other}"
            )))
        }
    };

    if msg_type == MSG_SERVER_ERROR {
        let text = String::from_utf8_lossy(&body).to_string();
        let code_part = error_code
            .map(|c| format!(" (code={c})"))
            .unwrap_or_default();
        return Err(SttError::ProviderUnavailable(format!(
            "Volcengine server error{code_part}: {text}"
        )));
    }

    Ok(ParsedFrame {
        msg_type,
        flags,
        serialization,
        sequence,
        body,
    })
}

fn parse_response(session_id: &str, frame: &ParsedFrame) -> Option<TranscriptDelta> {
    if frame.msg_type != MSG_SERVER_RESPONSE || frame.serialization != SER_JSON {
        return None;
    }
    let value: Value = serde_json::from_slice(&frame.body).ok()?;
    let result = value.get("result")?;
    // Per the BigModel docs, `utterances[].definite=true` is only set
    // when the server is explicitly configured with `end_window_size`
    // (silence-based endpointing) — plain streaming sessions never see
    // it. The reliable EOS signal is the negative-sequence flag the
    // server returns alongside the terminal frame, mirroring the
    // negated seq we send on the last audio chunk.
    let is_terminal_frame = (frame.flags & FLAG_NEGATIVE) != 0;
    if let Some(utts) = result.get("utterances").and_then(|v| v.as_array()) {
        if let Some(latest) = utts.last() {
            let text = latest.get("text").and_then(|v| v.as_str()).unwrap_or("");
            if text.is_empty() {
                return None;
            }
            let definite = latest
                .get("definite")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let start_ms = latest.get("start_time").and_then(|v| v.as_u64());
            let end_ms = latest.get("end_time").and_then(|v| v.as_u64());
            return Some(TranscriptDelta {
                session_id: session_id.to_string(),
                text: text.to_string(),
                is_final: definite || is_terminal_frame,
                start_ms,
                end_ms,
                confidence: None,
                language: None,
                accumulated: None,
            });
        }
    }
    let text = result.get("text").and_then(|v| v.as_str()).unwrap_or("");
    if text.is_empty() {
        return None;
    }
    Some(TranscriptDelta {
        session_id: session_id.to_string(),
        text: text.to_string(),
        is_final: is_terminal_frame,
        start_ms: None,
        end_ms: None,
        confidence: None,
        language: None,
        accumulated: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_and_parse_frame_roundtrip_json_gzip_no_sequence() {
        let body = br#"{"hello":"world"}"#;
        let bytes =
            build_frame(MSG_SERVER_RESPONSE, 0, None, SER_JSON, COMPRESS_GZIP, body).unwrap();
        let parsed = parse_frame(&bytes).unwrap();
        assert_eq!(parsed.msg_type, MSG_SERVER_RESPONSE);
        assert_eq!(parsed.serialization, SER_JSON);
        assert_eq!(parsed.sequence, None);
        assert_eq!(parsed.body, body);
    }

    #[test]
    fn build_and_parse_audio_frame_with_sequence_uncompressed() {
        let body: Vec<u8> = (0..4096).map(|i| (i & 0xff) as u8).collect();
        let bytes = build_frame(
            MSG_CLIENT_AUDIO,
            FLAG_SEQUENCE,
            Some(42),
            SER_NONE,
            COMPRESS_NONE,
            &body,
        )
        .unwrap();
        let parsed = parse_frame(&bytes).unwrap();
        assert_eq!(parsed.msg_type, MSG_CLIENT_AUDIO);
        assert_eq!(parsed.flags & FLAG_SEQUENCE, FLAG_SEQUENCE);
        assert_eq!(parsed.serialization, SER_NONE);
        assert_eq!(parsed.sequence, Some(42));
        assert_eq!(parsed.body, body);
    }

    #[test]
    fn build_terminal_audio_frame_carries_negative_sequence() {
        // Mirror `RequestBuilder.new_audio_only_request(seq, ..., is_last=True)`:
        // flags = POS_SEQUENCE | NEG_SEQUENCE (0b0011), seq is negated.
        let bytes = build_frame(
            MSG_CLIENT_AUDIO,
            FLAG_SEQUENCE | FLAG_NEGATIVE,
            Some(-17),
            SER_NONE,
            COMPRESS_GZIP,
            b"",
        )
        .unwrap();
        let parsed = parse_frame(&bytes).unwrap();
        assert_eq!(parsed.flags, FLAG_SEQUENCE | FLAG_NEGATIVE);
        assert_eq!(parsed.sequence, Some(-17));
    }

    #[test]
    fn parse_server_frame_with_event_marker_skips_event_id() {
        // Server ack after config: flags=POS_SEQUENCE|EVENT, layout
        // [header][seq][event_id][payload_size][gzip(json)]. Pre-fix we
        // mis-read event_id as payload_size and rejected the frame with
        // "body shorter than declared size".
        let mut bytes = vec![
            0x11, // version=1, header_size=1
            (MSG_SERVER_RESPONSE << 4) | (FLAG_SEQUENCE | FLAG_EVENT),
            (SER_JSON << 4) | COMPRESS_GZIP,
            0, // reserved
        ];
        bytes.extend_from_slice(&1i32.to_be_bytes()); // sequence
        bytes.extend_from_slice(&50i32.to_be_bytes()); // event id (arbitrary)
        let payload = r#"{"result":{"text":"你好"}}"#.as_bytes();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        std::io::Write::write_all(&mut encoder, payload).unwrap();
        let gz = encoder.finish().unwrap();
        bytes.extend_from_slice(&(gz.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&gz);

        let parsed = parse_frame(&bytes).unwrap();
        assert_eq!(parsed.msg_type, MSG_SERVER_RESPONSE);
        assert_eq!(parsed.sequence, Some(1));
        assert_eq!(parsed.body, payload);
    }

    #[test]
    fn build_and_parse_server_response_with_sequence_gzip() {
        // Per BigModel docs, full server responses carry a positive
        // sequence echoing the client's audio frame index.
        let body = br#"{"result":{"text":"hello"}}"#;
        let bytes = build_frame(
            MSG_SERVER_RESPONSE,
            FLAG_SEQUENCE,
            Some(7),
            SER_JSON,
            COMPRESS_GZIP,
            body,
        )
        .unwrap();
        let parsed = parse_frame(&bytes).unwrap();
        assert_eq!(parsed.msg_type, MSG_SERVER_RESPONSE);
        assert_eq!(parsed.sequence, Some(7));
        assert_eq!(parsed.body, body);
    }

    #[test]
    fn frame_header_first_byte_is_one_one() {
        let bytes =
            build_frame(MSG_CLIENT_CONFIG, 0, None, SER_JSON, COMPRESS_GZIP, b"{}").unwrap();
        assert_eq!(bytes[0], 0x11);
    }

    #[test]
    fn parse_error_msg_type_surfaces_provider_unavailable() {
        // Server error frame carries a 4-byte error code before the
        // payload size (see `ResponseParser` in the demo). Hand-build
        // the layout: [header][code][size][gzip(body)].
        let mut bytes = vec![
            0x11,
            MSG_SERVER_ERROR << 4,
            (SER_JSON << 4) | COMPRESS_GZIP,
            0,
        ];
        bytes.extend_from_slice(&45000001i32.to_be_bytes()); // error code
        let body = b"app key invalid";
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        std::io::Write::write_all(&mut encoder, body).unwrap();
        let gz = encoder.finish().unwrap();
        bytes.extend_from_slice(&(gz.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&gz);

        let err = parse_frame(&bytes).unwrap_err();
        match err {
            SttError::ProviderUnavailable(msg) => {
                assert!(msg.contains("app key invalid"));
                assert!(msg.contains("45000001"), "error code should surface in msg");
            }
            other => panic!("expected ProviderUnavailable, got {other:?}"),
        }
    }

    #[test]
    fn parse_response_returns_partial_from_utterance() {
        let frame = ParsedFrame {
            msg_type: MSG_SERVER_RESPONSE,
            flags: 0,
            serialization: SER_JSON,
            sequence: None,
            body: r#"{"result":{"text":"你好世界","utterances":[
                {"text":"你好","definite":false,"start_time":0,"end_time":500}
            ]}}"#
                .as_bytes()
                .to_vec(),
        };
        let d = parse_response("s", &frame).unwrap();
        assert_eq!(d.text, "你好");
        assert!(!d.is_final);
        assert_eq!(d.start_ms, Some(0));
        assert_eq!(d.end_ms, Some(500));
    }

    #[test]
    fn parse_response_marks_final_when_definite() {
        let frame = ParsedFrame {
            msg_type: MSG_SERVER_RESPONSE,
            flags: 0,
            serialization: SER_JSON,
            sequence: None,
            body: r#"{"result":{"utterances":[{"text":"完成","definite":true}]}}"#
                .as_bytes()
                .to_vec(),
        };
        let d = parse_response("s", &frame).unwrap();
        assert!(d.is_final);
        assert_eq!(d.text, "完成");
    }

    #[test]
    fn parse_response_falls_back_to_rolling_text() {
        let frame = ParsedFrame {
            msg_type: MSG_SERVER_RESPONSE,
            flags: 0,
            serialization: SER_JSON,
            sequence: None,
            body: br#"{"result":{"text":"hello"}}"#.to_vec(),
        };
        let d = parse_response("s", &frame).unwrap();
        assert_eq!(d.text, "hello");
        assert!(!d.is_final);
    }
}
