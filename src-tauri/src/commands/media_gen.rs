//! Tauri command surface for the unified media-generation subsystem.
//!
//! Thin pass-through to `ha_core::media_gen`: provider CRUD, per-function
//! default chains, tool defaults, templates catalog, voices, connectivity
//! probe, and the sanitized overview. Config writes go through the crud
//! helpers (serialized `mutate_config`) on the blocking pool.

use crate::commands::CmdError;
use ha_core::media_gen::{
    AudioGenDefaults, ImageGenDefaults, MediaFunction, MediaGenOverview, MediaModelChain,
    MediaProviderConfig, MediaProviderTemplate,
};
use serde::Serialize;

fn media_err(err: ha_core::media_gen::crud::MediaWriteError) -> CmdError {
    CmdError::from(anyhow::anyhow!("{err}"))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaGenConfigView {
    pub providers: Vec<MediaProviderConfig>,
    pub chains: ha_core::media_gen::MediaDefaultChains,
    pub image_defaults: ImageGenDefaults,
    pub audio_defaults: AudioGenDefaults,
}

/// Full media-gen config for the settings panels. Desktop webview is in the
/// same trust domain as the local user, so providers come back **unmasked**
/// (matches `get_stt_providers` / LLM `get_providers`); the HTTP route keeps
/// masking on for remote clients.
#[tauri::command]
pub async fn get_media_gen_config() -> Result<MediaGenConfigView, CmdError> {
    let cfg = ha_core::config::cached_config();
    Ok(MediaGenConfigView {
        providers: cfg.media_gen.providers.clone(),
        chains: cfg.media_gen.chains.clone(),
        image_defaults: cfg.media_gen.image_defaults.clone(),
        audio_defaults: cfg.media_gen.audio_defaults.clone(),
    })
}

#[tauri::command]
pub async fn add_media_provider(
    provider: MediaProviderConfig,
) -> Result<MediaProviderConfig, CmdError> {
    ha_core::blocking::run_blocking(move || {
        ha_core::media_gen::crud::add_media_provider(provider, "ui").map_err(media_err)
    })
    .await
}

#[tauri::command]
pub async fn update_media_provider(provider: MediaProviderConfig) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || {
        ha_core::media_gen::crud::update_media_provider(provider, "ui").map_err(media_err)
    })
    .await
}

/// Returns true when a default chain referenced the deleted provider (the
/// chain slots were cleaned up in the same write).
#[tauri::command]
pub async fn delete_media_provider(provider_id: String) -> Result<bool, CmdError> {
    ha_core::blocking::run_blocking(move || {
        ha_core::media_gen::crud::delete_media_provider(provider_id, "ui").map_err(media_err)
    })
    .await
}

#[tauri::command]
pub async fn reorder_media_providers(provider_ids: Vec<String>) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || {
        ha_core::media_gen::crud::reorder_media_providers(provider_ids, "ui").map_err(media_err)
    })
    .await
}

/// `function` ∈ image | speech | music | sfx；`chain = null` 清回自动档。
#[tauri::command]
pub async fn set_media_default_chain(
    function: String,
    chain: Option<MediaModelChain>,
) -> Result<(), CmdError> {
    let function = MediaFunction::parse(&function)
        .ok_or_else(|| CmdError::from(anyhow::anyhow!("unknown media function: {function}")))?;
    ha_core::blocking::run_blocking(move || {
        ha_core::media_gen::crud::set_media_default_chain(function, chain, "ui").map_err(media_err)
    })
    .await
}

#[tauri::command]
pub async fn update_media_gen_defaults(
    image_defaults: ImageGenDefaults,
    audio_defaults: AudioGenDefaults,
) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || {
        ha_core::media_gen::crud::update_media_gen_defaults(image_defaults, audio_defaults, "ui")
            .map_err(media_err)
    })
    .await
}

/// Built-in vendor templates + preset models (GUI-only catalog).
#[tauri::command]
pub async fn get_media_provider_templates() -> Result<Vec<MediaProviderTemplate>, CmdError> {
    Ok(ha_core::media_gen::media_provider_templates())
}

/// Voice catalog for one configured provider (ElevenLabs live fetch with a
/// 10-minute fingerprint cache; OpenAI-style vendors return the static
/// documented voice names).
#[tauri::command]
pub async fn list_media_voices(
    provider_id: String,
    limit: Option<u32>,
) -> Result<Vec<ha_core::media_gen::voices::VoiceOption>, CmdError> {
    ha_core::media_gen::voices::list_media_voices(&provider_id, limit.unwrap_or(100))
        .await
        .map_err(Into::into)
}

/// Connectivity probe ("Test connection"). Accepts either a saved
/// `providerId` or a pre-save draft (`kind` + `apiKey` + `baseUrl`) —
/// flattened args so both transports share the same frontend call shape.
/// Ok/Err both carry the probe-result JSON (Err = failure).
#[tauri::command]
pub async fn test_media_provider(
    provider_id: Option<String>,
    kind: Option<ha_core::media_gen::MediaVendorKind>,
    api_key: Option<String>,
    base_url: Option<String>,
) -> Result<String, String> {
    ha_core::media_gen::probe::test_media_provider(
        ha_core::media_gen::probe::TestMediaProviderInput {
            provider_id,
            kind,
            api_key,
            base_url,
        },
    )
    .await
}

/// Sanitized availability / capability view (no credentials) for the design
/// dialogs and the tool-settings chain hints.
#[tauri::command]
pub async fn get_media_gen_overview() -> Result<MediaGenOverview, CmdError> {
    let cfg = ha_core::config::cached_config();
    Ok(ha_core::media_gen::media_gen_overview(&cfg.media_gen))
}
