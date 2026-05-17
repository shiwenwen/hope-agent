//! STT subsystem HTTP routes.
//!
//! Mirrors the Tauri command surface in `src-tauri/src/commands/stt.rs`.
//! Phase 1 covers provider CRUD, active / fallback / IM-fallback selection,
//! known local backend catalog with probe + one-click upsert, and one-shot
//! batch transcription.

use axum::extract::Path;
use axum::Json;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use ha_core::stt::{
    self, ActiveSttModel, AudioPayload, KnownLocalSttBackend, SttModelConfig, SttProviderConfig,
    SttSessionManager, SttWriteError, Transcript, TranscriptOptions, MAX_BATCH_AUDIO_BYTES,
};

use crate::error::AppError;

fn stt_write_error(err: SttWriteError) -> AppError {
    match err {
        SttWriteError::NotFound(_) | SttWriteError::ModelNotFound { .. } => {
            AppError::not_found(err.to_string())
        }
        SttWriteError::UnknownLocalBackend(_) | SttWriteError::IncapableForBatch { .. } => {
            AppError::bad_request(err.to_string())
        }
        SttWriteError::Config(err) => AppError::internal(err.to_string()),
    }
}

// ── Provider CRUD ─────────────────────────────────────────────────

/// `GET /api/stt/providers`
pub async fn list_stt_providers() -> Result<Json<Vec<SttProviderConfig>>, AppError> {
    let cfg = ha_core::config::cached_config();
    Ok(Json(cfg.stt.providers.iter().map(|p| p.masked()).collect()))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderBody {
    pub provider: SttProviderConfig,
}

/// `POST /api/stt/providers` — body `{ "provider": { ... } }` to match the
/// Tauri command shape (HTTP transport sends the same args object verbatim).
pub async fn add_stt_provider(
    Json(body): Json<ProviderBody>,
) -> Result<Json<SttProviderConfig>, AppError> {
    let masked = stt::add_stt_provider(body.provider, "http").map_err(stt_write_error)?;
    Ok(Json(masked))
}

/// `PUT /api/stt/providers/{providerId}` — body wrapped in `{ provider }`.
/// Path id wins on mismatch (defensive against stale body ids).
pub async fn update_stt_provider(
    Path(provider_id): Path<String>,
    Json(mut body): Json<ProviderBody>,
) -> Result<Json<Value>, AppError> {
    body.provider.id = provider_id;
    stt::update_stt_provider(body.provider, "http").map_err(stt_write_error)?;
    Ok(Json(json!({ "updated": true })))
}

/// `DELETE /api/stt/providers/{id}`
pub async fn delete_stt_provider(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    let touched_active = stt::delete_stt_provider(id, "http").map_err(stt_write_error)?;
    Ok(Json(
        json!({ "deleted": true, "touchedActive": touched_active }),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderBody {
    pub provider_ids: Vec<String>,
}

/// `POST /api/stt/providers/reorder`
pub async fn reorder_stt_providers(Json(body): Json<ReorderBody>) -> Result<Json<Value>, AppError> {
    stt::reorder_stt_providers(body.provider_ids, "http").map_err(stt_write_error)?;
    Ok(Json(json!({ "reordered": true })))
}

// ── Active model selection ────────────────────────────────────────

/// `GET /api/stt/active-model`
pub async fn get_active_stt_model() -> Result<Json<Value>, AppError> {
    let cfg = ha_core::config::cached_config();
    Ok(Json(json!({ "activeModel": cfg.stt.active_model })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetActiveBody {
    pub provider_id: String,
    pub model_id: String,
}

/// `PUT /api/stt/active-model`
pub async fn set_active_stt_model(
    Json(body): Json<SetActiveBody>,
) -> Result<Json<SttProviderConfig>, AppError> {
    let provider = stt::set_active_stt_model(body.provider_id, body.model_id, "http")
        .map_err(stt_write_error)?;
    Ok(Json(provider.masked()))
}

/// `DELETE /api/stt/active-model`
pub async fn clear_active_stt_model() -> Result<Json<Value>, AppError> {
    stt::clear_active_stt_model("http").map_err(stt_write_error)?;
    Ok(Json(json!({ "cleared": true })))
}

/// `GET /api/stt/fallback-models`
pub async fn get_stt_fallback_models() -> Result<Json<Vec<ActiveSttModel>>, AppError> {
    let cfg = ha_core::config::cached_config();
    Ok(Json(cfg.stt.fallback_models.clone()))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FallbackBody {
    pub chain: Vec<ActiveSttModel>,
}

/// `PUT /api/stt/fallback-models`
pub async fn set_stt_fallback_models(
    Json(body): Json<FallbackBody>,
) -> Result<Json<Value>, AppError> {
    stt::set_stt_fallback_models(body.chain, "http").map_err(stt_write_error)?;
    Ok(Json(json!({ "updated": true })))
}

/// `GET /api/stt/im-fallback-model`
pub async fn get_im_fallback_stt_model() -> Result<Json<Value>, AppError> {
    let cfg = ha_core::config::cached_config();
    Ok(Json(
        json!({ "imFallbackModel": cfg.stt.im_fallback_model }),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImFallbackBody {
    pub selection: Option<ActiveSttModel>,
}

/// `PUT /api/stt/im-fallback-model`
pub async fn set_im_fallback_stt_model(
    Json(body): Json<ImFallbackBody>,
) -> Result<Json<Value>, AppError> {
    stt::set_im_fallback_stt_model(body.selection, "http").map_err(stt_write_error)?;
    Ok(Json(json!({ "updated": true })))
}

// ── Local backend catalog ─────────────────────────────────────────

/// `GET /api/stt/local-backends`
pub async fn list_local_stt_backends() -> Result<Json<Vec<KnownLocalSttBackend>>, AppError> {
    Ok(Json(stt::known_local_stt_backends()))
}

/// `GET /api/stt/local-backends/{key}/probe`
pub async fn probe_local_stt_backend(Path(key): Path<String>) -> Result<Json<Value>, AppError> {
    let backend = stt::known_local_stt_backend(&key)
        .ok_or_else(|| AppError::not_found(format!("Unknown STT backend: {key}")))?;
    let alive = stt::probe_local_backend_alive(&backend).await;
    Ok(Json(json!({ "alive": alive })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertLocalSttBody {
    pub provider: SttProviderConfig,
    pub model: SttModelConfig,
    #[serde(default)]
    pub activate: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertLocalSttResponse {
    pub provider_id: String,
    pub model_id: String,
}

/// `POST /api/stt/local-backends/{backendKey}/upsert`
pub async fn upsert_local_stt_provider(
    Path(backend_key): Path<String>,
    Json(body): Json<UpsertLocalSttBody>,
) -> Result<Json<UpsertLocalSttResponse>, AppError> {
    let (provider_id, model_id) = stt::upsert_known_local_stt_provider(
        &backend_key,
        body.provider,
        body.model,
        body.activate,
        "http",
    )
    .map_err(stt_write_error)?;
    Ok(Json(UpsertLocalSttResponse {
        provider_id,
        model_id,
    }))
}

// ── Transcription ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscribeBlobBody {
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    pub mime_type: String,
    pub filename: String,
    pub base64: String,
    #[serde(default)]
    pub options: TranscriptOptions,
}

/// `POST /api/stt/transcribe` — one-shot batch transcription (JSON body
/// with base64 audio; multipart upload is a Phase 2 follow-up).
pub async fn stt_transcribe_blob(
    Json(body): Json<TranscribeBlobBody>,
) -> Result<Json<Transcript>, AppError> {
    // Pre-flight cap on base64 length (~4/3 of the decoded bytes) so a
    // hostile or runaway client can't allocate a 100MB Vec just to be
    // rejected after the fact. Hard cap == OpenAI's whisper-1 limit.
    let max_base64_len = (MAX_BATCH_AUDIO_BYTES.saturating_mul(4) / 3).saturating_add(4);
    if body.base64.len() > max_base64_len {
        return Err(AppError::bad_request(format!(
            "Audio payload exceeds {} MB cap",
            MAX_BATCH_AUDIO_BYTES / (1024 * 1024)
        )));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&body.base64)
        .map_err(|e| AppError::bad_request(format!("Invalid base64 audio payload: {e}")))?;
    if bytes.len() > MAX_BATCH_AUDIO_BYTES {
        return Err(AppError::bad_request(format!(
            "Decoded audio payload exceeds {} MB cap",
            MAX_BATCH_AUDIO_BYTES / (1024 * 1024)
        )));
    }

    let (primary, fallback) = match (body.provider_id, body.model_id) {
        (Some(p), Some(m)) => (
            Some(ActiveSttModel {
                provider_id: p,
                model_id: m,
            }),
            Vec::new(),
        ),
        _ => stt::current_desktop_chain(),
    };

    let payload = AudioPayload::Bytes {
        mime_type: body.mime_type,
        bytes,
        filename: body.filename,
    };

    let transcript = stt::failover_transcribe_batch(primary, fallback, payload, &body.options)
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    Ok(Json(transcript))
}

// ── Streaming session ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartSessionBody {
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub options: TranscriptOptions,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartSessionResponse {
    pub session_id: String,
}

/// `POST /api/stt/sessions`
pub async fn stt_start_session(
    Json(body): Json<StartSessionBody>,
) -> Result<Json<StartSessionResponse>, AppError> {
    let session_id = SttSessionManager::global()
        .start(body.provider_id, body.model_id, body.options)
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    Ok(Json(StartSessionResponse { session_id }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushChunkBody {
    pub base64: String,
}

/// Cap on a single streaming chunk so a misbehaving client can't push
/// arbitrary blobs through the realtime path. Matches the Deepgram WS
/// frame size cap (1 MiB) the upstream protocol expects.
const MAX_PUSH_CHUNK_BYTES: usize = 1024 * 1024;

/// `POST /api/stt/sessions/{id}/chunk`
pub async fn stt_push_chunk(
    Path(session_id): Path<String>,
    Json(body): Json<PushChunkBody>,
) -> Result<Json<Value>, AppError> {
    let max_base64_len = (MAX_PUSH_CHUNK_BYTES.saturating_mul(4) / 3).saturating_add(4);
    if body.base64.len() > max_base64_len {
        return Err(AppError::bad_request(format!(
            "Audio chunk exceeds {} KB cap",
            MAX_PUSH_CHUNK_BYTES / 1024
        )));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&body.base64)
        .map_err(|e| AppError::bad_request(format!("Invalid base64 chunk: {e}")))?;
    if bytes.len() > MAX_PUSH_CHUNK_BYTES {
        return Err(AppError::bad_request(format!(
            "Decoded audio chunk exceeds {} KB cap",
            MAX_PUSH_CHUNK_BYTES / 1024
        )));
    }
    SttSessionManager::global()
        .push_chunk(&session_id, bytes)
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    Ok(Json(json!({ "pushed": true })))
}

/// `POST /api/stt/sessions/{id}/finalize`
pub async fn stt_finalize_session(
    Path(session_id): Path<String>,
) -> Result<Json<Transcript>, AppError> {
    let transcript = SttSessionManager::global()
        .finalize(&session_id)
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    Ok(Json(transcript))
}

/// `DELETE /api/stt/sessions/{id}`
pub async fn stt_cancel_session(Path(session_id): Path<String>) -> Result<Json<Value>, AppError> {
    SttSessionManager::global()
        .cancel(&session_id)
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    Ok(Json(json!({ "cancelled": true })))
}
