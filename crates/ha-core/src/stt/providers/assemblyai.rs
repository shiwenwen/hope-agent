//! AssemblyAI Universal Streaming (v3) WebSocket transcription.
//!
//! Reference docs (AsyncAPI 2.6 spec embedded on these pages):
//! - <https://www.assemblyai.com/docs/speech-to-text/universal-streaming>
//! - <https://assemblyai.com/docs/api-reference/streaming-api/streaming-api>
//!
//! Endpoint: `wss://streaming.assemblyai.com/v3/ws`
//! Auth header: `Authorization: <api_key>` (raw key, NO `Bearer` / `Token`
//! prefix; per AsyncAPI `channels./v3/ws.bindings.ws.headers`).
//!
//! Query params per the spec (any not listed are server-defaulted):
//! - `speech_model`: `universal-streaming-english` (default) /
//!   `universal-streaming-multilingual` / `whisper-rt`. This is the
//!   real "model id" surface — we wire the GUI's `model.id` here.
//! - `encoding`: `pcm_s16le` (default) / `pcm_mulaw`.
//! - `format_turns`: string `"true"` / `"false"` (default `"false"`).
//!   When true, formatted final transcripts come on `end_of_turn=true`.
//! - `sample_rate`: integer (default 16000).
//! - `language`: ISO code, only meaningful with `speech_model=whisper-rt`.
//! - `language_detection`: `"true"` / `"false"`, only meaningful with
//!   `speech_model=universal-streaming-multilingual`.
//!
//! There is NO `language_code` query param — that name exists only on
//! the server-side `Turn` response payload. Sending it on the URL is
//! silently ignored (pre-fix bug).
//!
//! Wire shape:
//! - Upstream: raw PCM16 LE mono frames as binary WS messages (audio
//!   chunks 50-1000 ms per the spec — front-end ships 100 ms frames).
//! - Downstream: JSON text frames:
//!   - `{"type":"Begin","id":"...","expires_at":...}` — session bootstrap
//!   - `{"type":"Turn", turn_order, turn_is_formatted, end_of_turn,
//!      transcript, utterance, language_code?, language_confidence?,
//!      speaker_label?, end_of_turn_confidence, words[]}`
//!   - `{"type":"Termination", audio_duration_seconds,
//!      session_duration_seconds}`
//! - Client end-of-stream: `{"type":"Terminate"}` text frame.

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

use crate::provider::AuthProfile;
use crate::stt::errors::{SttError, SttResult};
use crate::stt::types::{SttModelConfig, SttProviderConfig, TranscriptDelta, TranscriptOptions};

/// Default sample rate when the caller didn't pin one. AssemblyAI
/// Universal Streaming expects 16 kHz PCM16 mono unless otherwise told.
const DEFAULT_SAMPLE_RATE_HZ: u32 = 16_000;

pub async fn open_stream(
    provider: &SttProviderConfig,
    model: &SttModelConfig,
    profile: &AuthProfile,
    options: &TranscriptOptions,
) -> SttResult<super::SttStream> {
    let base = provider.resolve_base_url(profile).trim_end_matches('/');
    let sample_rate = options
        .sample_rate_hz
        .filter(|hz| *hz > 0)
        .unwrap_or(DEFAULT_SAMPLE_RATE_HZ);
    // `speech_model` is the real model selector — drives English-only
    // vs multilingual vs Whisper-RT engine. Fall back to the English
    // default rather than emitting an empty value (server rejects empty
    // enum values).
    let speech_model = if model.id.trim().is_empty() {
        "universal-streaming-english"
    } else {
        model.id.trim()
    };
    let mut url = format!(
        "{}/v3/ws?sample_rate={}&speech_model={}",
        base,
        sample_rate,
        urlencoding::encode(speech_model),
    );
    // `format_turns=true` returns one formatted text per turn (vs
    // per-word partials). `end_of_turn` still distinguishes partial vs
    // final. Server expects this as the string "true", not the JSON bool.
    url.push_str("&format_turns=true");
    // Language handling is model-conditional per the AsyncAPI spec:
    // - whisper-rt: pass `language=<iso>` to pin to a specific language.
    // - universal-streaming-multilingual: enable auto-detection so
    //   `language_code` flows out on Turn payloads; the user-supplied
    //   `options.language` is informational here (no pinning param).
    // - universal-streaming-english: language is fixed; ignore input.
    if let Some(lang) = options
        .language
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if speech_model == "whisper-rt" {
            url.push_str(&format!("&language={}", urlencoding::encode(lang)));
        } else if speech_model == "universal-streaming-multilingual" {
            url.push_str("&language_detection=true");
        }
    }

    let https_twin = super::ws_to_https_twin(&url, "AssemblyAI")?;
    provider.check_ssrf(&https_twin).await?;

    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|e| SttError::Other(format!("Invalid AssemblyAI URL: {e}")))?;
    // AssemblyAI takes the raw API key without a `Bearer` / `Token`
    // prefix — that's their documented Universal Streaming auth shape.
    request.headers_mut().insert(
        "Authorization",
        profile
            .api_key
            .parse()
            .map_err(|e| SttError::Other(format!("Bad AssemblyAI auth header: {e}")))?,
    );

    let ws = super::ws_connect_with_caps(request, "AssemblyAI").await?;
    let (mut ws_sink, mut ws_stream) = ws.split();

    let (audio_tx, mut audio_rx) = mpsc::channel::<Vec<u8>>(super::STT_STREAM_CHANNEL_CAPACITY);
    let (delta_tx, delta_rx) =
        mpsc::channel::<Result<TranscriptDelta, SttError>>(super::STT_STREAM_CHANNEL_CAPACITY);

    tokio::spawn(async move {
        while let Some(chunk) = audio_rx.recv().await {
            if ws_sink.send(Message::Binary(chunk.into())).await.is_err() {
                break;
            }
        }
        // Send the Terminate sentinel and let AssemblyAI deliver its
        // final Turn + Termination frames before closing on its end.
        // Sending Close here races against those final data frames.
        let _ = ws_sink
            .send(Message::Text(r#"{"type":"Terminate"}"#.into()))
            .await;
    });

    let session_id = String::new();
    tokio::spawn(async move {
        while let Some(msg) = ws_stream.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Some(delta) = parse_message(&session_id, &text) {
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
                        .send(Err(SttError::Network(format!("AssemblyAI WS recv: {e}"))))
                        .await;
                    break;
                }
            }
        }
    });

    Ok(super::SttStream { audio_tx, delta_rx })
}

fn parse_message(session_id: &str, raw: &str) -> Option<TranscriptDelta> {
    let value: Value = serde_json::from_str(raw).ok()?;
    let msg_type = value.get("type").and_then(|v| v.as_str())?;
    if msg_type != "Turn" {
        return None;
    }
    let text = value
        .get("transcript")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if text.is_empty() {
        return None;
    }
    let is_final = value
        .get("end_of_turn")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // Per the AsyncAPI spec, `Turn` carries `end_of_turn_confidence`
    // (probability that this marks the end of an utterance), NOT
    // `transcript_confidence`. The pre-fix code looked for the latter
    // and silently returned None. The end-of-turn score is the closest
    // available signal — surface it through `confidence` so consumers
    // get something rather than nothing.
    let confidence = value
        .get("end_of_turn_confidence")
        .and_then(|v| v.as_f64())
        .map(|c| c as f32);
    // Multilingual / Whisper-RT runs populate `language_code` on each
    // Turn so downstream UI can pin the language label to what the
    // server detected.
    let language = value
        .get("language_code")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some(TranscriptDelta {
        session_id: session_id.to_string(),
        text: text.to_string(),
        is_final,
        start_ms: None,
        end_ms: None,
        confidence,
        language,
        accumulated: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_partial_turn() {
        let raw = r#"{
            "type":"Turn",
            "turn_order":1,
            "transcript":"hello world",
            "end_of_turn":false,
            "end_of_turn_confidence":0.12
        }"#;
        let d = parse_message("sess-x", raw).expect("Turn should parse");
        assert_eq!(d.text, "hello world");
        assert!(!d.is_final);
        // end_of_turn_confidence ~0.12 is low — that's correct for a
        // partial. We pipe it through as `confidence` for any UI that
        // wants it.
        assert!(d.confidence.unwrap() > 0.0);
    }

    #[test]
    fn parse_marks_final_on_end_of_turn() {
        let raw = r#"{"type":"Turn","transcript":"done","end_of_turn":true,"end_of_turn_confidence":0.95}"#;
        let d = parse_message("s", raw).unwrap();
        assert!(d.is_final);
        assert!(d.confidence.unwrap() > 0.9);
    }

    #[test]
    fn parse_extracts_language_code_for_multilingual() {
        let raw = r#"{"type":"Turn","transcript":"hola","end_of_turn":true,"language_code":"es"}"#;
        let d = parse_message("s", raw).unwrap();
        assert_eq!(d.language.as_deref(), Some("es"));
    }

    #[test]
    fn parse_skips_begin_and_termination() {
        assert!(parse_message("s", r#"{"type":"Begin","id":"abc"}"#).is_none());
        assert!(parse_message("s", r#"{"type":"Termination"}"#).is_none());
    }

    #[test]
    fn parse_skips_empty_transcript() {
        let raw = r#"{"type":"Turn","transcript":"","end_of_turn":false}"#;
        assert!(parse_message("s", raw).is_none());
    }
}
