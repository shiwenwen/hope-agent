//! Application configuration — the root structure persisted to `~/.hope-agent/config.json`.
//!
//! Historically named `ProviderStore`, this type actually owns the entire
//! user-facing config (providers, channels, memory, skills, tools, UI, server…).
//! It was renamed to `AppConfig` to match its real scope.
//!
//! The on-disk JSON shape is unchanged — all fields use `#[serde(rename_all = "camelCase")]`
//! and no wrapper struct is involved, so the Rust type name has zero impact on serialization.

pub(crate) mod autosave;
mod persistence;

pub(crate) use persistence::clear_legacy_server_token_without_backup;
pub(crate) use persistence::encode_model_eval_codex_secret;
#[doc(hidden)]
pub use persistence::reload_config_snapshot_from_disk;
#[cfg(any(test, feature = "test-support"))]
pub use persistence::replace_cache_for_test;
pub use persistence::{
    cached_config, config_health, initialize_model_eval_provider_secrets, load_config,
    mutate_config, mutate_config_async, reload_cache_from_disk, save_config, ConfigHealth,
    ModelEvalCodexCredential,
};
pub(crate) use persistence::{register_side_effects, ConfigSideEffects};
// 类型已下沉 ha-config-schema（含文末纯类型测试），glob 再导出保持
// `crate::config::Xxx` 全部既有路径不变；persistence（cached_config /
// mutate_config 读写 contract）留在本 crate。
pub use ha_config_schema::config::*;

/// Read-facing deferred-tools contract. Legacy on-disk configurations can
/// omit `mode`; normalize only the returned snapshot so every UI transport
/// displays the same policy that [`DeferredToolsConfig::effective_mode`]
/// enforces at runtime without silently migrating the file.
pub fn deferred_tools_config_for_read() -> DeferredToolsConfig {
    let mut config = cached_config().deferred_tools.clone();
    config.mode = Some(config.effective_mode());
    config
}
