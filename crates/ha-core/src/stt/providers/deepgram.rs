//! Deepgram realtime WebSocket STT.
//!
//! Reference docs (verified against the AsyncAPI 2.6 spec embedded on
//! the doc pages):
//! - <https://developers.deepgram.com/reference/speech-to-text-api/listen-streaming>
//! - <https://developers.deepgram.com/docs/live-streaming-audio>
//!
//! Endpoint: `wss://api.deepgram.com/v1/listen` (path-only; query string
//! configures the recognition).
//! Auth header: `Authorization: Token <api_key>` (the literal `Token`
//! prefix is required per docs).
//!
//! Query params we set:
//! - `model`: enum from spec — `nova-3` / `nova-3-general` /
//!   `nova-3-medical` / `nova-2` / `nova-2-meeting` / … (defaults to
//!   `nova-3` when the user didn't fill the model picker).
//! - `encoding=linear16` + `sample_rate=16000`: front-end AudioWorklet
//!   emits raw little-endian PCM16 mono, with no container header for
//!   Deepgram to sniff.
//! - `interim_results=true`: get partial transcripts.
//! - `smart_format=true`: Deepgram's current recommended formatter —
//!   superset of `punctuate`, also handles numerals / dates / proper
//!   nouns. We send it by default unless the user explicitly disables
//!   `options.punctuation`.
//! - `diarize=true` when the caller requests speaker labels.
//! - `language=<iso>` when the caller pins a language.
//!
//! Wire shape:
//! - Upstream: raw audio bytes as binary WS frames.
//! - Downstream: JSON text frames. The `Results` payload carries
//!   `channel.alternatives[0].transcript`, `is_final` (transcript is
//!   stable, won't be revised) and `speech_final` (utterance ended via
//!   VAD endpointing). Other server messages: `Metadata`,
//!   `UtteranceEnd`, `SpeechStarted` — informational, we ignore them.
//! - Client control frames: `{"type":"CloseStream"}` to end the
//!   session, `{"type":"Finalize"}` to force-flush a partial,
//!   `{"type":"KeepAlive"}` to bump the inactivity timer.

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

use crate::provider::AuthProfile;
use crate::stt::errors::{SttError, SttResult};
use crate::stt::types::{
    SttModelConfig, SttProviderConfig, Transcript, TranscriptDelta, TranscriptOptions,
};

/// Open a Deepgram streaming session. Returns `(audio_tx, delta_rx)`:
/// callers push raw audio bytes into `audio_tx` and drain transcript
/// deltas off `delta_rx`. Closing `audio_tx` (drop) signals end-of-audio;
/// the engine drains the upstream WS until its final delta then closes
/// `delta_rx`.
///
/// The two background tasks (audio → WS upstream / WS downstream → delta_rx)
/// share a `tokio_util::sync::CancellationToken`-equivalent flag via the
/// channels themselves — drop either end and the task notices.
pub async fn open_stream(
    provider: &SttProviderConfig,
    model: &SttModelConfig,
    profile: &AuthProfile,
    options: &TranscriptOptions,
) -> SttResult<super::SttStream> {
    let base = provider.resolve_base_url(profile).trim_end_matches('/');
    // PCM16 LE 16 kHz mono is the hope-agent worklet contract — declare
    // it explicitly so Deepgram doesn't try (and fail) to sniff a
    // container header from raw PCM frames.
    let sample_rate = options.sample_rate_hz.unwrap_or(16_000);
    // Empty `model=` is rejected by the server. Fall back to nova-3 (the
    // currently recommended default per Deepgram's `ListenV1Model`
    // enum) so a brand-new provider without a selected model still
    // connects.
    let model_id = if model.id.trim().is_empty() {
        "nova-3"
    } else {
        model.id.trim()
    };
    let mut url = format!(
        "{}/v1/listen?model={}&encoding=linear16&sample_rate={}&interim_results=true",
        base,
        urlencoding::encode(model_id),
        sample_rate
    );
    if let Some(lang) = &options.language {
        if !lang.is_empty() {
            url.push_str(&format!("&language={}", urlencoding::encode(lang)));
        }
    }
    // `smart_format` is the current recommended formatter — supersedes
    // raw `punctuate` and adds numeral / date / proper-noun handling.
    // Drive both off the same option toggle; when the user actively
    // disables punctuation, we also turn smart_format off (otherwise
    // the server still capitalises and adds periods).
    if options.punctuation.unwrap_or(true) {
        url.push_str("&smart_format=true&punctuate=true");
    }
    if options.diarization.unwrap_or(false) {
        url.push_str("&diarize=true");
    }

    let https_twin = super::ws_to_https_twin(&url, "Deepgram")?;
    provider.check_ssrf(&https_twin).await?;

    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|e| SttError::Other(format!("Invalid Deepgram URL: {e}")))?;
    request.headers_mut().insert(
        "Authorization",
        format!("Token {}", profile.api_key)
            .parse()
            .map_err(|e| SttError::Other(format!("Bad auth header: {e}")))?,
    );

    let ws = super::ws_connect_with_caps(request, "Deepgram").await?;
    let (mut ws_sink, mut ws_stream) = ws.split();

    let (audio_tx, mut audio_rx) = mpsc::channel::<Vec<u8>>(super::STT_STREAM_CHANNEL_CAPACITY);
    let (delta_tx, delta_rx) =
        mpsc::channel::<Result<TranscriptDelta, SttError>>(super::STT_STREAM_CHANNEL_CAPACITY);

    // Audio uplink — forward raw bytes from caller into WS binary frames.
    // When the caller drops `audio_tx`, audio_rx.recv() returns None →
    // we send Deepgram's "close stream" sentinel `{"type":"CloseStream"}`
    // and let the downstream task drain remaining final transcripts.
    tokio::spawn(async move {
        while let Some(chunk) = audio_rx.recv().await {
            if ws_sink.send(Message::Binary(chunk.into())).await.is_err() {
                break;
            }
        }
        // Send Deepgram's `CloseStream` sentinel and let the server
        // deliver its final transcript before closing. Sending Close
        // here races against the trailing partial→final frame.
        let _ = ws_sink
            .send(Message::Text(r#"{"type":"CloseStream"}"#.into()))
            .await;
    });

    // Session_id is filled in by the SttSessionManager after open_stream
    // returns — we don't need it on the engine side. Use empty placeholder.
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
                | Ok(Message::Pong(_)) => {
                    // ignore
                }
                Ok(Message::Close(_)) => break,
                Err(e) => {
                    let _ = delta_tx
                        .send(Err(SttError::Network(format!("Deepgram WS recv: {e}"))))
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
    let alt = value.pointer("/channel/alternatives/0")?;
    let text = alt.get("transcript").and_then(|v| v.as_str()).unwrap_or("");
    if text.is_empty() {
        return None;
    }
    let is_final = value
        .get("is_final")
        .and_then(|v| v.as_bool())
        .or_else(|| value.get("speech_final").and_then(|v| v.as_bool()))
        .unwrap_or(false);
    let confidence = alt
        .get("confidence")
        .and_then(|v| v.as_f64())
        .map(|c| c as f32);
    let start = value.get("start").and_then(|v| v.as_f64());
    let duration = value.get("duration").and_then(|v| v.as_f64());
    let start_ms = start.map(|s| (s * 1000.0).max(0.0) as u64);
    let end_ms = match (start, duration) {
        (Some(s), Some(d)) => Some(((s + d) * 1000.0).max(0.0) as u64),
        _ => None,
    };
    Some(TranscriptDelta {
        session_id: session_id.to_string(),
        text: text.to_string(),
        is_final,
        start_ms,
        end_ms,
        confidence,
        language: None,
        accumulated: None,
    })
}

/// Batch path for Deepgram — placeholder so the failover chain stays
/// uniform across providers. Real `/v1/listen` HTTP batch can be wired in
/// when needed.
#[allow(dead_code)]
pub async fn transcribe_batch(
    _provider: &SttProviderConfig,
    _model: &SttModelConfig,
    _profile: &AuthProfile,
    _audio: crate::stt::AudioPayload,
    _options: &TranscriptOptions,
) -> SttResult<Transcript> {
    Err(SttError::Other(
        "Deepgram batch transcription is not implemented; use the streaming session instead".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_partial_transcript() {
        let raw = r#"{
            "channel":{"alternatives":[{"transcript":"hello","confidence":0.97}]},
            "is_final":false,
            "start":1.2,
            "duration":0.4
        }"#;
        let d = parse_message("sess-1", raw).expect("partial should parse");
        assert_eq!(d.text, "hello");
        assert!(!d.is_final);
        assert_eq!(d.start_ms, Some(1200));
        assert_eq!(d.end_ms, Some(1600));
        assert!(d.confidence.unwrap() > 0.9);
        assert_eq!(d.session_id, "sess-1");
    }

    #[test]
    fn parse_marks_final_via_is_final() {
        let raw = r#"{"channel":{"alternatives":[{"transcript":"hi"}]},"is_final":true}"#;
        let d = parse_message("s", raw).unwrap();
        assert!(d.is_final);
    }

    #[test]
    fn parse_falls_back_to_speech_final() {
        let raw = r#"{"channel":{"alternatives":[{"transcript":"bye"}]},"speech_final":true}"#;
        let d = parse_message("s", raw).unwrap();
        assert!(d.is_final);
    }

    #[test]
    fn parse_skips_empty_transcripts() {
        let raw = r#"{"channel":{"alternatives":[{"transcript":""}]},"is_final":false}"#;
        assert!(parse_message("s", raw).is_none());
    }
}
