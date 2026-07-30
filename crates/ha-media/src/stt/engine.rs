//! failover 批量转写真实现（kernel `ha_core::stt::failover_transcribe_batch`
//! trampoline 经 wire() 指向这里）。

use ha_core::config::cached_config;
use ha_core::provider::AuthProfile;
use ha_core::stt::errors::{SttError, SttResult};
use ha_core::stt::types::{
    ActiveSttModel, AudioPayload, SttModelConfig, SttProviderConfig, SttProviderKind, Transcript,
    TranscriptOptions,
};
use ha_core::stt::{resolve_active, AttemptedModel, FailoverError};

use crate::stt::providers;

/// Batch transcribe one audio payload through the engine that backs a
/// specific `(provider, model, profile)` triple.
pub async fn transcribe_with(
    provider: &SttProviderConfig,
    model: &SttModelConfig,
    profile: &AuthProfile,
    audio: AudioPayload,
    options: &TranscriptOptions,
    session_id: Option<&str>,
) -> SttResult<Transcript> {
    if !provider.enabled {
        return Err(SttError::NotFound(format!(
            "Provider {} is disabled",
            provider.id
        )));
    }
    let audio_bytes = match &audio {
        AudioPayload::Bytes { bytes, .. } => Some(bytes.len() as u64),
        AudioPayload::File { path, .. } => std::fs::metadata(path).ok().map(|m| m.len()),
    };
    let started = std::time::Instant::now();
    let result = match provider.kind {
        SttProviderKind::OpenaiTranscriptions | SttProviderKind::OpenaiCompatible => {
            providers::openai::transcribe_batch(provider, model, profile, audio, options).await
        }
        SttProviderKind::OpenaiChatCompletionsAsr => {
            providers::chat_completions_asr::transcribe_batch(
                provider, model, profile, audio, options,
            )
            .await
        }
        SttProviderKind::ElevenlabsStt => {
            providers::elevenlabs::transcribe_batch(provider, model, profile, audio, options).await
        }
        SttProviderKind::XaiStt => {
            providers::xai::transcribe_batch(provider, model, profile, audio, options).await
        }
        SttProviderKind::DeepgramWs
        | SttProviderKind::AssemblyaiWs
        | SttProviderKind::AzureWs
        | SttProviderKind::VolcengineWs
        | SttProviderKind::XunfeiWs => Err(SttError::Other(format!(
            "Batch transcription is not supported for provider kind {:?}; use the streaming session instead",
            provider.kind
        ))),
    };
    let duration_ms = started.elapsed().as_millis() as u64;
    let mut event = ha_core::model_usage::ModelUsageEvent::new(ha_core::model_usage::KIND_STT);
    event.operation = Some("stt.transcribe_batch".to_string());
    event.source = Some("stt".to_string());
    event.provider_id = Some(provider.id.clone());
    event.provider_name = Some(provider.name.clone());
    event.model_id = Some(model.id.clone());
    event.session_id = session_id.map(|s| s.to_string());
    event.duration_ms = Some(duration_ms);
    event.success = result.is_ok();
    event.error = result.as_ref().err().map(|e| e.to_string());
    event.metadata = Some(serde_json::json!({
        "provider_kind": provider.kind.display_name(),
        "audio_bytes": audio_bytes,
        "language": &options.language,
        "transcript_duration_ms": result.as_ref().ok().and_then(|t| t.duration_ms),
    }));
    ha_core::model_usage::record_model_usage_best_effort(event);
    result
}

/// Try `primary`, then each entry in `fallback`, until one succeeds or the
/// chain is exhausted. Hard errors (`UnsupportedAudio`) short-circuit — no
/// point retrying when the audio itself is the problem.
pub async fn failover_transcribe_batch(
    primary: Option<ActiveSttModel>,
    fallback: Vec<ActiveSttModel>,
    audio: AudioPayload,
    options: &TranscriptOptions,
    session_id: Option<&str>,
) -> Result<Transcript, FailoverError> {
    let chain: Vec<ActiveSttModel> = primary.into_iter().chain(fallback).collect();
    if chain.is_empty() {
        return Err(FailoverError {
            attempts: Vec::new(),
            terminal: SttError::NoActiveModel,
        });
    }

    let cfg = cached_config();
    let mut attempts = Vec::new();
    let mut last_error: Option<SttError> = None;
    let last_idx = chain.len() - 1;
    let mut audio = Some(audio);

    for (idx, active) in chain.iter().enumerate() {
        let Some((provider, model, profile)) = resolve_active(&cfg, active) else {
            let err = SttError::NotFound(active.to_string());
            attempts.push(AttemptedModel {
                provider_id: active.provider_id.clone(),
                model_id: active.model_id.clone(),
                error_code: err.code(),
                error_message: err.to_string(),
            });
            last_error = Some(err);
            continue;
        };

        // Final attempt consumes the original payload; earlier attempts
        // clone so retries can reuse it. Audio payloads are typically a
        // few MB so the saved allocation is worth the bookkeeping.
        let audio_for_attempt = if idx == last_idx {
            audio.take().expect("audio still owned on last attempt")
        } else {
            audio.as_ref().expect("audio still owned").clone()
        };
        match transcribe_with(
            &provider,
            &model,
            &profile,
            audio_for_attempt,
            options,
            session_id,
        )
        .await
        {
            Ok(transcript) => return Ok(transcript),
            Err(err) => {
                let retriable = err.is_retriable();
                attempts.push(AttemptedModel {
                    provider_id: active.provider_id.clone(),
                    model_id: active.model_id.clone(),
                    error_code: err.code(),
                    error_message: err.to_string(),
                });
                if !retriable {
                    return Err(FailoverError {
                        attempts,
                        terminal: err,
                    });
                }
                last_error = Some(err);
            }
        }
    }

    Err(FailoverError {
        attempts,
        terminal: last_error.unwrap_or(SttError::NoActiveModel),
    })
}
