//! DashScope / Qwen-style ASR via OpenAI chat-completions `input_audio`.
//!
//! Reference docs:
//! - <https://help.aliyun.com/zh/model-studio/qwen-speech-recognition> (中文)
//! - <https://www.alibabacloud.com/help/en/model-studio/qwen-speech-recognition> (English)
//! - <https://www.alibabacloud.com/help/en/model-studio/compatibility-of-openai-with-dashscope>
//!
//! Alibaba's Qwen3-ASR family does NOT expose the standard
//! `/v1/audio/transcriptions` multipart endpoint that
//! [`super::openai`] targets. Instead the audio is sent as an
//! `input_audio` content block inside a regular `chat/completions`
//! request, and the transcript falls out as the assistant message body.
//!
//! Request shape (DashScope compatible-mode):
//! ```json
//! POST {base_url}/v1/chat/completions
//! {
//!   "model": "qwen3-asr-flash",
//!   "messages": [
//!     {
//!       "role": "user",
//!       "content": [
//!         {
//!           "type": "input_audio",
//!           "input_audio": {
//!             "data": "<base64 of audio bytes>",
//!             "format": "opus" | "mp3" | "wav" | ...
//!           }
//!         }
//!       ]
//!     }
//!   ]
//! }
//! ```
//! Response is the usual chat-completions envelope; we read
//! `choices[0].message.content` (string).
//!
//! See: <https://www.alibabacloud.com/help/en/model-studio/qwen-speech-recognition>

use std::time::Duration;

use base64::Engine;

use crate::provider::{apply_proxy, AuthProfile};

use crate::stt::errors::{SttError, SttResult};
use crate::stt::types::{
    AudioPayload, SttModelConfig, SttProviderConfig, SttProviderKind, Transcript, TranscriptOptions,
};

use super::{classify_http_status, classify_reqwest_error, load_batch_audio};

const REQUEST_TIMEOUT_SECS: u64 = 120;

fn chat_completions_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    format!("{}/v1/chat/completions", trimmed)
}

/// One-shot transcription via DashScope `chat/completions` + `input_audio`.
pub async fn transcribe_batch(
    provider: &SttProviderConfig,
    model: &SttModelConfig,
    profile: &AuthProfile,
    audio: AudioPayload,
    _options: &TranscriptOptions,
) -> SttResult<Transcript> {
    debug_assert!(matches!(
        provider.kind,
        SttProviderKind::OpenaiChatCompletionsAsr
    ));

    let base_url = provider.resolve_base_url(profile);
    let url = chat_completions_url(base_url);

    // SSRF check and audio load are independent; parallelize to overlap
    // DNS / TCP work with disk I/O for `File` payloads.
    let (ssrf_result, audio_result) =
        tokio::join!(provider.check_ssrf(&url), load_batch_audio(audio));
    ssrf_result?;
    let (bytes, mime_type, _filename) = audio_result?;
    let format = audio_format_hint(&mime_type);
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    // DashScope rejects raw base64 with `InternalError.Algo.InvalidParameter:
    // The provided URL does not appear to be valid` — it parses `data` as a
    // URL/URI. The data-URI form `data:audio/<format>;base64,<b64>` works for
    // both DashScope and (per OpenAI multimodal docs) `gpt-4o-audio-preview`,
    // so this is the safe cross-vendor encoding.
    let data_uri = format!("data:audio/{};base64,{}", format, encoded);

    let body = serde_json::json!({
        "model": model.id,
        "messages": [
            {
                "role": "user",
                "content": [
                    {
                        "type": "input_audio",
                        "input_audio": {
                            "data": data_uri,
                            "format": format,
                        }
                    }
                ]
            }
        ]
    });

    let client = apply_proxy(
        reqwest::Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .user_agent("hope-agent/stt")
            .redirect(reqwest::redirect::Policy::none()),
    )
    .build()
    .map_err(|e| SttError::Network(format!("HTTP client build failed: {e}")))?;

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", profile.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| classify_reqwest_error(&e))?;

    let status = response.status();
    if status.is_redirection() {
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("<unknown>")
            .to_string();
        return Err(SttError::SsrfBlocked(format!(
            "STT provider redirected ({status}) to {location}; redirects are disabled to prevent SSRF bypass"
        )));
    }
    let body_text = response
        .text()
        .await
        .map_err(|e| SttError::Network(e.to_string()))?;

    if !status.is_success() {
        return Err(classify_http_status(status, &body_text));
    }

    parse_chat_completions_transcript(provider, model, &body_text)
}

/// Map a mime-type to the short `format` token DashScope expects.
///
/// DashScope documents `wav | mp3 | m4a | opus | flac | ogg | aac`. The
/// browser MediaRecorder default on Chromium is `audio/webm;codecs=opus`
/// — DashScope accepts that as `opus` (it sniffs the container internally).
fn audio_format_hint(mime_type: &str) -> &'static str {
    let m = mime_type.to_ascii_lowercase();
    if m.contains("opus") {
        return "opus";
    }
    if m.contains("webm") {
        // Browser webm is virtually always opus-in-webm. DashScope sniffs.
        return "opus";
    }
    if m.contains("wav") {
        return "wav";
    }
    if m.contains("mp3") || m.contains("mpeg") {
        return "mp3";
    }
    if m.contains("m4a") || m.contains("mp4") || m.contains("aac") {
        return "m4a";
    }
    if m.contains("flac") {
        return "flac";
    }
    if m.contains("ogg") {
        return "ogg";
    }
    // Fall back to opus — closest match for unknown browser blobs.
    "opus"
}

fn parse_chat_completions_transcript(
    provider: &SttProviderConfig,
    model: &SttModelConfig,
    body: &str,
) -> SttResult<Transcript> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| SttError::Other(format!("Invalid JSON from provider: {e}")))?;

    // `choices[0].message.content` may be either a plain string OR (per
    // OpenAI multimodal spec) an array of content parts. Handle both.
    let content = value
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .ok_or_else(|| {
            SttError::Other("Provider response missing `choices[0].message.content`".to_string())
        })?;

    let text = if let Some(s) = content.as_str() {
        s.to_string()
    } else if let Some(arr) = content.as_array() {
        arr.iter()
            .filter_map(|part| part.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("")
    } else {
        return Err(SttError::Other(format!(
            "Unexpected `message.content` shape: {content:?}"
        )));
    };

    Ok(Transcript {
        text,
        language: None,
        duration_ms: None,
        segments: Vec::new(),
        provider_id: provider.id.clone(),
        model_id: model.id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_completions_url_trims_trailing_slash() {
        assert_eq!(
            chat_completions_url("https://dashscope.aliyuncs.com/compatible-mode/"),
            "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://dashscope.aliyuncs.com/compatible-mode"),
            "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions"
        );
    }

    #[test]
    fn audio_format_hint_maps_common_mimes() {
        assert_eq!(audio_format_hint("audio/webm;codecs=opus"), "opus");
        assert_eq!(audio_format_hint("audio/ogg"), "ogg");
        assert_eq!(audio_format_hint("audio/wav"), "wav");
        assert_eq!(audio_format_hint("audio/mpeg"), "mp3");
        assert_eq!(audio_format_hint("audio/mp4"), "m4a");
        assert_eq!(audio_format_hint("audio/flac"), "flac");
        // Unknown falls back to opus (browser default).
        assert_eq!(audio_format_hint("audio/x-unknown"), "opus");
    }

    #[test]
    fn parse_chat_completions_extracts_plain_string_content() {
        let provider = SttProviderConfig::new(
            "DashScope",
            SttProviderKind::OpenaiChatCompletionsAsr,
            "https://dashscope.aliyuncs.com/compatible-mode",
        );
        let model = SttModelConfig::new("qwen3-asr-flash", "Qwen3-ASR Flash");
        let body = r#"{
            "choices": [
                { "message": { "role": "assistant", "content": "你好世界" } }
            ]
        }"#;
        let t = parse_chat_completions_transcript(&provider, &model, body).unwrap();
        assert_eq!(t.text, "你好世界");
        assert_eq!(t.provider_id, provider.id);
        assert_eq!(t.model_id, "qwen3-asr-flash");
    }

    #[test]
    fn parse_chat_completions_extracts_array_content() {
        let provider = SttProviderConfig::new(
            "DashScope",
            SttProviderKind::OpenaiChatCompletionsAsr,
            "https://dashscope.aliyuncs.com/compatible-mode",
        );
        let model = SttModelConfig::new("qwen3-asr-flash", "Qwen3-ASR Flash");
        let body = r#"{
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": [
                            { "type": "text", "text": "hello " },
                            { "type": "text", "text": "world" }
                        ]
                    }
                }
            ]
        }"#;
        let t = parse_chat_completions_transcript(&provider, &model, body).unwrap();
        assert_eq!(t.text, "hello world");
    }

    #[test]
    fn parse_chat_completions_rejects_missing_content() {
        let provider = SttProviderConfig::new(
            "DashScope",
            SttProviderKind::OpenaiChatCompletionsAsr,
            "https://dashscope.aliyuncs.com/compatible-mode",
        );
        let model = SttModelConfig::new("qwen3-asr-flash", "Qwen3-ASR Flash");
        let err =
            parse_chat_completions_transcript(&provider, &model, r#"{"choices":[]}"#).unwrap_err();
        assert_eq!(err.code(), "other");
    }
}
