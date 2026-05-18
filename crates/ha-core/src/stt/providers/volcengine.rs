//! Volcengine (火山引擎 / 字节豆包) streaming ASR WebSocket — BigModel
//! generation (`/api/v3/sauc/bigmodel`).
//!
//! Reference docs:
//! - <https://www.volcengine.com/docs/6561/1354869> (大模型流式语音识别 API)
//! - <https://www.volcengine.com/docs/6561/1096680> (豆包语音 API 接口文档)
//!
//! WS upgrade headers (matches the volcengine-asr-ha reference impl —
//! these names are what the BigModel server actually accepts; the public
//! docs are inconsistent about the exact name of the last one):
//! - `X-Api-App-Key: <app_key>` — `extra.app_key` (the "APP ID" digit
//!   string in the Volcengine console)
//! - `X-Api-Access-Key: <access_key>` — `provider.api_key` (the
//!   "Access Token" — NOT the IAM Secret Key)
//! - `X-Api-Resource-Id: <resource>` — defaults to the 1.0 hourly
//!   resource `volc.bigasr.sauc.duration`. For the 2.0 "Seed" tier
//!   (instances whose id contains `Speech_Recognition_Seed_streaming…`),
//!   override `extra.resource_id` to `volc.seedasr.sauc.duration`.
//! - `X-Api-Connect-Id: <UUID>` — fresh per session. **Not**
//!   `X-Api-Request-Id` (the latter applies to the async REST submit/
//!   query family, not the WS upgrade).
//!
//! Sequence number — placed in each binary frame body (see
//! `FLAG_SEQUENCE` + the `seq` counter below), NOT in an HTTP header.
//! Frame 0x1 (config) carries no sequence; audio frames carry positive
//! seq; the terminal audio frame uses `FLAG_NEGATIVE` to mark EOS.
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

/// `flags & FLAG_SEQUENCE` indicates a 4-byte BE sequence number is
/// embedded in the frame between the fixed header and the payload size.
const FLAG_SEQUENCE: u8 = 0x1;
/// Last-packet marker (set on the final client audio frame).
const FLAG_NEGATIVE: u8 = 0x2;

const SER_JSON: u8 = 0x1;
const SER_RAW: u8 = 0xf;
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

    let request_id = Uuid::new_v4().simple().to_string();

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
    // Volcengine names this header `X-Api-Connect-Id` on the WS upgrade
    // (per the official SDK and reference implementations). Earlier
    // versions of this file used `X-Api-Request-Id`, which the server
    // silently rejected as a malformed upgrade → 400 Bad Request.
    // `X-Api-Sequence` is NOT a WS upgrade header — the sequence number
    // travels in the binary frame body (see FLAG_SEQUENCE / `seq` below).
    headers.insert(
        "X-Api-Connect-Id",
        request_id
            .parse()
            .map_err(|e| SttError::Other(format!("Bad X-Api-Connect-Id value: {e}")))?,
    );

    // Pre-connect diagnostic: ResourceId mismatch (BigASR 1.0 vs Seed 2.0)
    // is the #1 cause of an opaque 400 here. Logging the resolved values
    // (sans keys) makes the failure self-diagnosable from the log file.
    crate::app_info!(
        "stt",
        "volcengine::open_stream",
        "WS connect url={} resource_id={} app_key_len={} access_key_len={} connect_id={}",
        url,
        resource_id,
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
    let cfg_body = json!({
        "user": { "uid": request_id },
        "audio": {
            "format": "pcm",
            "sample_rate": sample_rate,
            "bits": 16,
            "channel": 1,
        },
        "request": {
            "model_name": model.id,
            "language": language,
            "enable_itn": true,
            "enable_punc": true,
        }
    });
    let config_frame = build_frame(
        MSG_CLIENT_CONFIG,
        0,
        None,
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
        // Audio frames skip gzip (compression=0x0): raw PCM16 doesn't
        // compress well (< 5% saving) and at 50 chunks/sec the encoder
        // cost dominates the wire saving. Volcengine accepts both
        // compressed and uncompressed audio frames.
        let mut seq: u32 = 1;
        while let Some(chunk) = audio_rx.recv().await {
            match build_frame(
                MSG_CLIENT_AUDIO,
                FLAG_SEQUENCE,
                Some(seq),
                SER_RAW,
                COMPRESS_NONE,
                &chunk,
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
        // EOS: empty audio frame with negative-sequence + last flag.
        if let Ok(frame) = build_frame(
            MSG_CLIENT_AUDIO,
            FLAG_SEQUENCE | FLAG_NEGATIVE,
            Some(seq),
            SER_RAW,
            COMPRESS_NONE,
            &[],
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
    sequence: Option<u32>,
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
    sequence: Option<u32>,
    body: Vec<u8>,
}

fn parse_frame(bytes: &[u8]) -> SttResult<ParsedFrame> {
    if bytes.len() < 8 {
        return Err(SttError::Other("Volcengine frame too short".into()));
    }
    let header_words = bytes[0] & 0x0f;
    let header_len = header_words as usize * 4;
    if header_len == 0 {
        return Err(SttError::Other(
            "Volcengine frame missing payload prefix".into(),
        ));
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
        let seq = u32::from_be_bytes([
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

    if bytes.len() < cursor + 4 {
        return Err(SttError::Other(
            "Volcengine frame missing payload size prefix".into(),
        ));
    }
    let body_size = u32::from_be_bytes([
        bytes[cursor],
        bytes[cursor + 1],
        bytes[cursor + 2],
        bytes[cursor + 3],
    ]) as usize;
    let body_start = cursor + 4;
    if bytes.len() < body_start + body_size {
        return Err(SttError::Other(
            "Volcengine frame body shorter than declared size".into(),
        ));
    }
    let raw_body = &bytes[body_start..body_start + body_size];

    let body = match compression {
        COMPRESS_GZIP => {
            let mut decoder = GzDecoder::new(raw_body);
            let mut out = Vec::new();
            decoder
                .read_to_end(&mut out)
                .map_err(|e| SttError::Other(format!("Volcengine gzip decode: {e}")))?;
            out
        }
        0 => raw_body.to_vec(),
        other => {
            return Err(SttError::Other(format!(
                "Volcengine unsupported compression {other}"
            )))
        }
    };

    if msg_type == MSG_SERVER_ERROR {
        let text = String::from_utf8_lossy(&body).to_string();
        return Err(SttError::ProviderUnavailable(format!(
            "Volcengine server error: {text}"
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
    // Prefer the most recent utterance for partial classification; fall
    // back to the rolling `text` field for cumulative final.
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
                is_final: definite,
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
        is_final: false,
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
            SER_RAW,
            COMPRESS_NONE,
            &body,
        )
        .unwrap();
        let parsed = parse_frame(&bytes).unwrap();
        assert_eq!(parsed.msg_type, MSG_CLIENT_AUDIO);
        assert_eq!(parsed.flags & FLAG_SEQUENCE, FLAG_SEQUENCE);
        assert_eq!(parsed.serialization, SER_RAW);
        assert_eq!(parsed.sequence, Some(42));
        assert_eq!(parsed.body, body);
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
        // Build a server-error frame (type=0xf) with a plaintext error
        // body so the parser surfaces the string via ProviderUnavailable.
        let body = b"app key invalid";
        let bytes = build_frame(MSG_SERVER_ERROR, 0, None, SER_JSON, COMPRESS_GZIP, body).unwrap();
        let err = parse_frame(&bytes).unwrap_err();
        match err {
            SttError::ProviderUnavailable(msg) => assert!(msg.contains("app key invalid")),
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
