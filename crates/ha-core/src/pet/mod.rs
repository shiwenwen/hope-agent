//! Desktop Pet core.
//!
//! The module owns the transport-neutral package format, validation, library
//! store and activity projection. It deliberately contains no Tauri types. The
//! activity projection reads only a bounded terminal-assistant preview for an
//! unread Ready turn; all other overlay state remains structured metadata.

pub mod activity;
pub mod asset;
pub mod atlas;
pub mod creator;
pub mod import;
pub mod store;
pub mod types;

pub use activity::activity_snapshot;
pub use activity::emit_activity_changed;
pub use asset::{read_installed_sprite, resolve_installed_sprite};
pub use creator::create_preview;
pub use import::{
    cancel_import_preview, commit_import, discover_codex_candidates, preview_import,
    preview_import_async, preview_thumbnail, preview_token_thumbnail,
};
pub use store::{delete_pet, export_codex_package, list_pets, restore_pet};
pub use types::*;

/// Patch the user-facing Pet configuration through the shared config mutation
/// contract. Callers must pass only the fields they intend to change: keeping
/// the merge inside the serialized mutation prevents an enable toggle from
/// rolling back a concurrent selection (and vice versa).
///
/// Selection changes also hold the cross-process library lock from validation
/// through persistence. `store::delete_pet` takes the same lock before checking
/// the selected ref, so config can never end up pointing at a removed package.
pub async fn update_config(
    enabled: Option<bool>,
    selected_pet_ref: Option<PetRef>,
    source: &'static str,
) -> anyhow::Result<PetConfig> {
    if selected_pet_ref
        .as_ref()
        .is_some_and(|pet_ref| !pet_ref.is_well_formed())
    {
        anyhow::bail!("pet_ref_invalid");
    }
    let event_config = crate::blocking::run_blocking(move || {
        let _library_guard = selected_pet_ref
            .as_ref()
            .map(|_| store::acquire_library_lock())
            .transpose()?;
        if let Some(selected) = selected_pet_ref.as_ref() {
            let available = list_pets()?.pets.iter().any(|pet| pet.pet_ref == *selected);
            if !available {
                anyhow::bail!("pet_not_found");
            }
        }
        crate::config::mutate_config(("pet", source), move |store| {
            if let Some(enabled) = enabled {
                store.pet.enabled = enabled;
            }
            if let Some(selected) = selected_pet_ref {
                store.pet.selected_pet_ref = selected;
            }
            Ok(store.pet.clone())
        })
    })
    .await?;
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(
            "pet:config_changed",
            serde_json::json!({
                "enabled": event_config.enabled,
                "selectedPetRef": event_config.selected_pet_ref,
                "source": source,
            }),
        );
    }
    Ok(event_config)
}
