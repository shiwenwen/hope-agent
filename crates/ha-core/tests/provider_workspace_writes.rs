//! Exercise public provider writes against a process-isolated temporary config.
//! One test keeps HA_DATA_DIR and the process-wide config cache in one lifecycle.

use ha_core::config::{cached_config, mutate_config};
use ha_core::provider::{
    add_and_activate_provider, add_many_providers, add_provider, update_provider, ApiType,
    AuthProfile, ModelConfig, ProviderConfig,
};

fn bound_provider(id: &str, base_url: &str, override_url: Option<&str>) -> ProviderConfig {
    let mut provider = ProviderConfig::new(
        id.into(),
        ApiType::Anthropic,
        base_url.into(),
        "synthetic-legacy-key".into(),
    );
    provider.id = id.into();
    provider.models = vec![ModelConfig {
        id: "m1".into(),
        name: "Synthetic model".into(),
        input_types: vec!["text".into()],
        context_window: 128_000,
        max_tokens: 1024,
        reasoning: false,
        thinking_style: None,
        cost_input: None,
        cost_output: None,
    }];
    provider.auth_profiles = vec![AuthProfile {
        id: format!("{id}-profile"),
        label: "Synthetic workspace".into(),
        api_key: "synthetic-profile-key".into(),
        base_url: override_url.map(str::to_owned),
        anthropic_workspace_id: Some("wrkspc_Test123".into()),
        enabled: true,
    }];
    provider
}

fn stored_provider(id: &str) -> ProviderConfig {
    cached_config()
        .providers
        .iter()
        .find(|provider| provider.id == id)
        .expect("provider was persisted")
        .clone()
}

#[test]
fn public_workspace_writes_sanitize_before_validation_and_reject_invalid_batches() {
    let temp = tempfile::tempdir().unwrap();
    std::env::set_var("HA_DATA_DIR", temp.path());
    assert_eq!(ha_core::paths::root_dir().unwrap(), temp.path());
    ha_core::paths::ensure_dirs().unwrap();
    mutate_config(("providers.test", "workspace-write-test"), |config| {
        config.providers.clear();
        config.active_model = None;
        config.fallback_models.clear();
        Ok(())
    })
    .unwrap();

    let added = add_provider(
        bound_provider("add", "\u{2003}https://api.anthropic.com\n", Some(" \t")),
        "workspace-write-test",
    )
    .unwrap();
    let stored = stored_provider(&added.provider.id);
    assert_eq!(stored.base_url, "https://api.anthropic.com");
    assert_eq!(stored.auth_profiles[0].base_url, None);

    let mut activate = bound_provider(
        "activate",
        " https://relay.example.invalid ",
        Some("\u{2003}https://api.anthropic.com\n"),
    );
    activate.models[0].id = " m1 ".into();
    let active_id =
        add_and_activate_provider(activate, " m1 ".into(), "workspace-write-test").unwrap();
    let active = cached_config().active_model.clone().unwrap();
    assert_eq!(active.provider_id, active_id);
    assert_eq!(active.model_id, "m1");
    assert_eq!(
        stored_provider(&active_id).auth_profiles[0]
            .base_url
            .as_deref(),
        Some("https://api.anthropic.com")
    );

    let ids = add_many_providers(
        vec![
            bound_provider("batch-a", "\u{2003}https://api.anthropic.com\n", None),
            bound_provider("batch-b", " https://api.anthropic.com ", Some("\t")),
        ],
        "workspace-write-test",
    )
    .unwrap();
    assert_eq!(ids, vec!["batch-a", "batch-b"]);
    for id in ids {
        let stored = stored_provider(&id);
        assert_eq!(stored.base_url, "https://api.anthropic.com");
        assert_eq!(stored.auth_profiles[0].base_url, None);
    }

    let mut updated = stored_provider(&added.provider.id).masked();
    updated.base_url = "\u{2003}https://api.anthropic.com\n".into();
    updated.auth_profiles[0].base_url = Some(" \t".into());
    update_provider(updated, "workspace-write-test").unwrap();
    let stored = stored_provider(&added.provider.id);
    assert_eq!(stored.base_url, "https://api.anthropic.com");
    assert_eq!(stored.auth_profiles[0].base_url, None);
    assert_eq!(stored.auth_profiles[0].api_key, "synthetic-profile-key");

    let config_path = ha_core::paths::config_path().unwrap();
    let before = std::fs::read(&config_path).unwrap();
    let invalid = bound_provider("invalid", " https://relay.example.invalid ", None);
    assert!(add_provider(invalid.clone(), "workspace-write-test").is_err());
    assert!(
        add_and_activate_provider(invalid.clone(), "m1".into(), "workspace-write-test").is_err()
    );
    assert!(add_many_providers(
        vec![
            bound_provider("must-not-appear", " https://api.anthropic.com ", None),
            invalid.clone(),
        ],
        "workspace-write-test",
    )
    .is_err());
    let mut invalid_update = invalid;
    invalid_update.id = added.provider.id;
    assert!(update_provider(invalid_update, "workspace-write-test").is_err());
    assert_eq!(std::fs::read(config_path).unwrap(), before);
    assert!(!cached_config()
        .providers
        .iter()
        .any(|provider| provider.id == "must-not-appear"));
}
