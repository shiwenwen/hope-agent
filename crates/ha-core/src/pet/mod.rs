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
    commit_import, discover_codex_candidates, preview_import, preview_import_async,
    preview_thumbnail, preview_token_thumbnail,
};
pub use store::{delete_pet, export_codex_package, list_pets, restore_pet};
pub use types::*;

/// Persist the user-facing pet selection through the shared config mutation
/// contract.  Library validation happens before the write so a stale Settings
/// view cannot select a package that another process has just removed.
pub async fn save_config(config: PetConfig, source: &'static str) -> anyhow::Result<()> {
    if !config.selected_pet_ref.is_well_formed() {
        anyhow::bail!("pet_ref_invalid");
    }
    let selected = config.selected_pet_ref.clone();
    let available = crate::blocking::run_blocking(move || {
        Ok::<_, anyhow::Error>(list_pets()?.pets.iter().any(|pet| pet.pet_ref == selected))
    })
    .await?;
    if !available {
        anyhow::bail!("pet_not_found");
    }
    let event_config = config.clone();
    crate::config::mutate_config_async(("pet", source), move |store| {
        store.pet = config;
        Ok(())
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
    Ok(())
}
