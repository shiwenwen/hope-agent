//! 桌面宠物特征 crate（阶段 4 第四刀，自 ha-core 迁出）：包格式与校验 /
//! sprite 库 store / 导入（含 Codex 兼容）/ creator / 活动投影。
//!
//! kernel 侧留存（`ha_core::pet`）：`ChatUiSurface`（chat_turns 表列
//! wire 类型）、`emit_activity_changed` + 活动修订计数、`update_config`
//! trampoline；活动候选行查询随表下沉
//! `ha_core::session::pet_activity`（特征侧不持 raw conn）。
//!
//! 装配契约与其它特征 crate 相同：每个调 `ha_core::init_runtime` 的二进
//! 制必须先调 [`wire()`]。The crate deliberately contains no Tauri
//! types. The activity projection reads only a bounded terminal-assistant
//! preview for an unread Ready turn; all other overlay state remains
//! structured metadata.

pub mod activity;
pub mod asset;
pub mod atlas;
pub mod creator;
pub mod import;
pub mod store;
pub mod types;

pub use activity::activity_snapshot;
// kernel 持有（写路径在 kernel），再导出保持既有路径。
pub use asset::{read_installed_sprite, resolve_installed_sprite};
pub use creator::create_preview;
pub use ha_core::pet::emit_activity_changed;
pub use import::{
    cancel_import_preview, commit_import, discover_codex_candidates, preview_import,
    preview_import_async, preview_thumbnail, preview_token_thumbnail,
};
pub use store::{delete_pet, export_codex_package, list_pets, restore_pet, upgrade_pet_to_v2};
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
    let event_config = ha_core::blocking::run_blocking(move || {
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
        ha_core::config::mutate_config(("pet", source), move |store| {
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
    if let Some(bus) = ha_core::globals::get_event_bus() {
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

/// Replace a selected pet only if it still matches the expected source. The
/// comparison and persisted update share the library lock with ordinary pet
/// selection, so an upgrade can never overwrite a newer user choice.
pub(crate) async fn replace_selected_pet_if(
    expected_pet_ref: PetRef,
    selected_pet_ref: PetRef,
    source: &'static str,
) -> anyhow::Result<bool> {
    let event_config = ha_core::blocking::run_blocking(move || {
        let _library_guard = store::acquire_library_lock()?;
        if ha_core::config::cached_config().pet.selected_pet_ref != expected_pet_ref {
            return Ok(None);
        }
        let available = list_pets()?
            .pets
            .iter()
            .any(|pet| pet.pet_ref == selected_pet_ref);
        if !available {
            anyhow::bail!("pet_not_found");
        }
        ha_core::config::mutate_config(("pet", source), move |store| {
            store.pet.selected_pet_ref = selected_pet_ref;
            Ok(store.pet.clone())
        })
        .map(Some)
    })
    .await?;
    let Some(event_config) = event_config else {
        return Ok(false);
    };
    if let Some(bus) = ha_core::globals::get_event_bus() {
        bus.emit(
            "pet:config_changed",
            serde_json::json!({
                "enabled": event_config.enabled,
                "selectedPetRef": event_config.selected_pet_ref,
                "source": source,
            }),
        );
    }
    Ok(true)
}

/// 幂等装配：注册 kernel 的 pet 配置更新钩子（`ha_core::pet::update_config`
/// trampoline 的真实现）。
pub fn wire() {
    static WIRED: std::sync::Once = std::sync::Once::new();
    WIRED.call_once(|| {
        fn config_updater(
            enabled: Option<bool>,
            selected_pet_ref: Option<PetRef>,
            source: &'static str,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = anyhow::Result<PetConfig>> + Send + 'static>,
        > {
            Box::pin(update_config(enabled, selected_pet_ref, source))
        }
        ha_core::pet::register_pet_config_updater(config_updater)
            .expect("ha_pet::wire() registers the pet config updater exactly once");
    });
}
