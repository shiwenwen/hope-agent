//! Tauri command surface for the STT subsystem.
//!
//! Thin pass-through to `ha_core::stt`. Phase 1 ships:
//! - Provider CRUD (cloud + local share one list)
//! - Active / fallback / IM-fallback selection
//! - Known local backend catalog (whisper.cpp / faster-whisper / FunASR /
//!   sherpa-onnx) + port probe + one-click upsert
//! - One-shot batch transcription (`stt_transcribe_blob`)
//!
//! Streaming sessions, IM auto-transcribe, and Tauri-side WebSocket
//! providers (Deepgram / AssemblyAI / Azure / Volcengine / iFlytek) ship in
//! later phases.

use crate::commands::CmdError;
use crate::AppState;
use base64::Engine;
use ha_core::stt::{
    failover_transcribe_batch, known_local_stt_backends, probe_local_backend_alive,
    upsert_known_local_stt_provider, ActiveSttModel, AudioPayload, KnownLocalSttBackend,
    SttModelConfig, SttProviderConfig, Transcript, TranscriptOptions,
};
use tauri::State;

// ── Provider CRUD ─────────────────────────────────────────────────

#[tauri::command]
pub async fn get_stt_providers(
    _state: State<'_, AppState>,
) -> Result<Vec<SttProviderConfig>, CmdError> {
    Ok(ha_core::config::cached_config()
        .stt
        .providers
        .iter()
        .map(|p| p.masked())
        .collect())
}

#[tauri::command]
pub async fn add_stt_provider(
    provider: SttProviderConfig,
    _state: State<'_, AppState>,
) -> Result<SttProviderConfig, CmdError> {
    ha_core::stt::add_stt_provider(provider, "ui").map_err(Into::into)
}

#[tauri::command]
pub async fn update_stt_provider(
    provider: SttProviderConfig,
    _state: State<'_, AppState>,
) -> Result<(), CmdError> {
    ha_core::stt::update_stt_provider(provider, "ui").map_err(Into::into)
}

#[tauri::command]
pub async fn delete_stt_provider(
    provider_id: String,
    _state: State<'_, AppState>,
) -> Result<bool, CmdError> {
    ha_core::stt::delete_stt_provider(provider_id, "ui").map_err(Into::into)
}

#[tauri::command]
pub async fn reorder_stt_providers(
    provider_ids: Vec<String>,
    _state: State<'_, AppState>,
) -> Result<(), CmdError> {
    ha_core::stt::reorder_stt_providers(provider_ids, "ui").map_err(Into::into)
}

// ── Active model selection ────────────────────────────────────────

#[tauri::command]
pub async fn get_active_stt_model(
    _state: State<'_, AppState>,
) -> Result<Option<ActiveSttModel>, CmdError> {
    Ok(ha_core::config::cached_config().stt.active_model.clone())
}

#[tauri::command]
pub async fn set_active_stt_model(
    provider_id: String,
    model_id: String,
    _state: State<'_, AppState>,
) -> Result<SttProviderConfig, CmdError> {
    let provider = ha_core::stt::set_active_stt_model(provider_id, model_id, "ui")?;
    Ok(provider.masked())
}

#[tauri::command]
pub async fn clear_active_stt_model(_state: State<'_, AppState>) -> Result<(), CmdError> {
    ha_core::stt::clear_active_stt_model("ui").map_err(Into::into)
}

#[tauri::command]
pub async fn get_stt_fallback_models(
    _state: State<'_, AppState>,
) -> Result<Vec<ActiveSttModel>, CmdError> {
    Ok(ha_core::config::cached_config().stt.fallback_models.clone())
}

#[tauri::command]
pub async fn set_stt_fallback_models(
    chain: Vec<ActiveSttModel>,
    _state: State<'_, AppState>,
) -> Result<(), CmdError> {
    ha_core::stt::set_stt_fallback_models(chain, "ui").map_err(Into::into)
}

#[tauri::command]
pub async fn get_im_fallback_stt_model(
    _state: State<'_, AppState>,
) -> Result<Option<ActiveSttModel>, CmdError> {
    Ok(ha_core::config::cached_config()
        .stt
        .im_fallback_model
        .clone())
}

#[tauri::command]
pub async fn set_im_fallback_stt_model(
    selection: Option<ActiveSttModel>,
    _state: State<'_, AppState>,
) -> Result<(), CmdError> {
    ha_core::stt::set_im_fallback_stt_model(selection, "ui").map_err(Into::into)
}

// ── Local backend catalog ─────────────────────────────────────────

#[tauri::command]
pub async fn list_known_local_stt_backends(
    _state: State<'_, AppState>,
) -> Result<Vec<KnownLocalSttBackend>, CmdError> {
    Ok(known_local_stt_backends())
}

#[tauri::command]
pub async fn probe_local_stt_backend(
    key: String,
    _state: State<'_, AppState>,
) -> Result<bool, CmdError> {
    let backend = ha_core::stt::known_local_stt_backend(&key)
        .ok_or_else(|| CmdError::msg(format!("Unknown STT backend: {key}")))?;
    Ok(probe_local_backend_alive(&backend).await)
}

#[tauri::command]
pub async fn upsert_known_local_stt_provider_cmd(
    backend_key: String,
    provider: SttProviderConfig,
    model: SttModelConfig,
    activate: bool,
    _state: State<'_, AppState>,
) -> Result<UpsertLocalSttResult, CmdError> {
    let (provider_id, model_id) =
        upsert_known_local_stt_provider(&backend_key, provider, model, activate, "ui")?;
    Ok(UpsertLocalSttResult {
        provider_id,
        model_id,
    })
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertLocalSttResult {
    pub provider_id: String,
    pub model_id: String,
}

// ── Transcription ─────────────────────────────────────────────────

#[tauri::command]
pub async fn stt_transcribe_blob(
    provider_id: Option<String>,
    model_id: Option<String>,
    mime_type: String,
    filename: String,
    base64: String,
    options: TranscriptOptions,
    _state: State<'_, AppState>,
) -> Result<Transcript, CmdError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&base64)
        .map_err(|e| CmdError::msg(format!("Invalid base64 audio payload: {e}")))?;

    let (primary, fallback) = match (provider_id, model_id) {
        (Some(p), Some(m)) => (
            Some(ActiveSttModel {
                provider_id: p,
                model_id: m,
            }),
            Vec::new(),
        ),
        _ => ha_core::stt::current_desktop_chain(),
    };

    let payload = AudioPayload::Bytes {
        mime_type,
        bytes,
        filename,
    };

    failover_transcribe_batch(primary, fallback, payload, &options)
        .await
        .map_err(|e| CmdError::msg(e.to_string()))
}
