use serde::Serialize;
use tauri::{Emitter, Manager};

use super::CmdError;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PetAssetPath {
    pub path: Option<String>,
    pub mime: String,
    pub etag: String,
}

#[tauri::command]
pub async fn get_pet_config_cmd() -> Result<ha_pet::PetConfig, CmdError> {
    Ok(ha_core::config::cached_config().pet.clone())
}

#[tauri::command]
pub async fn save_pet_config_cmd(config: ha_pet::PetConfig) -> Result<(), CmdError> {
    // This legacy-shaped command is the selection endpoint. The Settings
    // switch uses `pet_set_enabled_cmd`; preserving the live enabled field here
    // prevents the two controls from overwriting each other.
    ha_pet::update_config(None, Some(config.selected_pet_ref), "settings-ui").await?;
    Ok(())
}

#[tauri::command]
pub async fn pet_set_enabled_cmd(
    app: tauri::AppHandle,
    enabled: bool,
    source: Option<String>,
) -> Result<ha_pet::PetConfig, CmdError> {
    let source = match source.as_deref() {
        Some("slash-command") => "slash-command",
        Some("pet-window") => "pet-window",
        Some("sidebar") => "sidebar",
        _ => "settings-ui",
    };
    let previous_enabled = ha_core::config::cached_config().pet.enabled;
    let sync_before_persist = source != "pet-window" && previous_enabled != enabled;
    if sync_before_persist {
        // Validate native window creation/closure before making the desktop
        // switch durable. PetWindow itself is the exception: it must receive
        // the command response before the main renderer closes that WebView.
        crate::pet_window::sync_enabled(&app, enabled)?;
    }
    let config = match ha_pet::update_config(Some(enabled), None, source).await {
        Ok(config) => config,
        Err(error) => {
            if sync_before_persist {
                let _ = crate::pet_window::sync_enabled(&app, previous_enabled);
            }
            return Err(error.into());
        }
    };
    // PetWindow invokes this command itself. Closing that webview before its
    // response is delivered produces a false client error. The always-mounted
    // main renderer consumes pet:config_changed and performs the serialized
    // lifecycle sync after this command returns.
    Ok(config)
}

#[tauri::command]
pub async fn pet_sync_window_cmd(app: tauri::AppHandle) -> Result<(), CmdError> {
    crate::pet_window::sync_enabled(&app, ha_core::config::cached_config().pet.enabled)?;
    Ok(())
}

#[tauri::command]
pub async fn pet_list_cmd() -> Result<ha_pet::PetLibrarySnapshot, CmdError> {
    ha_core::blocking::run_blocking(ha_pet::list_pets)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn pet_asset_path_cmd(asset_id: String) -> Result<PetAssetPath, CmdError> {
    let descriptor =
        ha_core::blocking::run_blocking(move || ha_pet::resolve_installed_sprite(&asset_id))
            .await?;
    Ok(PetAssetPath {
        path: descriptor
            .path
            .map(|path| path.to_string_lossy().to_string()),
        mime: descriptor.mime.to_string(),
        etag: descriptor.etag,
    })
}

#[tauri::command]
pub async fn pet_codex_candidates_cmd() -> Result<ha_pet::PetCandidatePage, CmdError> {
    ha_core::blocking::run_blocking(ha_pet::discover_codex_candidates)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn pet_candidate_thumbnail_cmd(candidate_id: String) -> Result<Vec<u8>, CmdError> {
    ha_core::blocking::run_blocking(move || ha_pet::preview_thumbnail(&candidate_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn pet_preview_thumbnail_cmd(preview_token: String) -> Result<Vec<u8>, CmdError> {
    ha_core::blocking::run_blocking(move || ha_pet::preview_token_thumbnail(&preview_token))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn pet_create_preview_cmd(
    request: ha_pet::PetCreateRequest,
) -> Result<ha_pet::PetImportPreview, CmdError> {
    ha_pet::create_preview(request).await.map_err(Into::into)
}

#[tauri::command]
pub async fn pet_upgrade_v2_cmd(
    request: ha_pet::PetUpgradeRequest,
) -> Result<ha_pet::PetUpgradeResult, CmdError> {
    ha_pet::upgrade_pet_to_v2(request).await.map_err(Into::into)
}

#[tauri::command]
pub async fn pet_import_preview_cmd(
    request: ha_pet::PetImportPreviewRequest,
) -> Result<ha_pet::PetImportPreview, CmdError> {
    ha_pet::preview_import_async(request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn pet_import_preview_cancel_cmd(preview_token: String) -> Result<bool, CmdError> {
    ha_pet::cancel_import_preview(preview_token)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn pet_import_commit_cmd(
    request: ha_pet::PetImportCommitRequest,
) -> Result<ha_pet::PetImportCommitResult, CmdError> {
    ha_pet::commit_import(request).await.map_err(Into::into)
}

#[tauri::command]
pub async fn pet_delete_cmd(
    pet_ref: ha_pet::PetRef,
    expected_package_hash: String,
) -> Result<ha_pet::PetDeleteResult, CmdError> {
    ha_core::blocking::run_blocking(move || ha_pet::delete_pet(&pet_ref, &expected_package_hash))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn pet_restore_cmd(
    request: ha_pet::PetRestoreRequest,
) -> Result<ha_pet::PetSummary, CmdError> {
    ha_core::blocking::run_blocking(move || ha_pet::restore_pet(&request.restore_token))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn pet_export_cmd(pet_ref: ha_pet::PetRef) -> Result<ha_pet::PetExportResult, CmdError> {
    ha_core::blocking::run_blocking(move || ha_pet::export_codex_package(&pet_ref))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn pet_activity_snapshot_cmd(
    state: tauri::State<'_, crate::AppState>,
) -> Result<ha_pet::PetActivitySnapshot, CmdError> {
    ha_pet::activity_snapshot(state.session_db.clone())
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn pet_apply_window_bounds_cmd(
    app: tauri::AppHandle,
    request: crate::pet_window::PetWindowBoundsRequest,
) -> Result<crate::pet_window::PetWindowBoundsResult, CmdError> {
    let window = app
        .get_webview_window("pet")
        .ok_or_else(|| CmdError::msg("pet_window_unavailable"))?;
    crate::pet_window::apply_bounds(&window, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn pet_focus_target_cmd(
    app: tauri::AppHandle,
    target: Option<ha_pet::PetNavigationTarget>,
) -> Result<(), CmdError> {
    if let Some(main) = app.get_webview_window("main") {
        main.show()?;
        main.unminimize()?;
        main.set_focus()?;
    }
    if let Some(target) = target {
        app.emit("pet:navigate", target)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn pet_take_install_link_cmd() -> Result<Option<String>, CmdError> {
    Ok(crate::pet_deep_link::take_pending())
}
