use super::types::{EvalCredentialOption, EvalModelOption, EvalResolvedLaunch};
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use ha_eval_spec::app::{
    AppResolvedModelBinding, EvalAppRunRequest, NetworkEnforcement, RuntimeEnvironmentSnapshot,
};
use ha_eval_spec::digest_serializable;
use ha_eval_spec::model::ModelProfile;
use std::collections::BTreeMap;

/// Resolve owner-plane Provider/model references for legacy Coding/Domain
/// campaign executors. The returned configs may contain credentials and must
/// remain in memory: callers clear deprecated `providers` fields before
/// persisting or returning campaign input.
pub fn resolve_owner_provider_refs(
    references: &[(String, String, Option<String>)],
) -> Result<Vec<crate::provider::ProviderConfig>> {
    if references.is_empty() {
        return Ok(Vec::new());
    }
    let source = crate::config::load_config()?;
    let mut resolved = BTreeMap::<String, crate::provider::ProviderConfig>::new();
    for (provider_id, model_id, credential_profile_ref) in references {
        let provider = source
            .providers
            .iter()
            .find(|candidate| candidate.id == *provider_id && candidate.enabled)
            .ok_or_else(|| anyhow!("selected evaluation Provider is missing or disabled"))?;
        if !provider.models.iter().any(|model| model.id == *model_id) {
            bail!("selected evaluation model is not registered by its Provider");
        }
        let mut provider = provider.clone();
        let selected_profile = if let Some(profile_id) = credential_profile_ref {
            Some(
                provider
                    .auth_profiles
                    .iter()
                    .find(|profile| profile.id == *profile_id && profile.enabled)
                    .ok_or_else(|| {
                        anyhow!("selected Provider credential profile is missing or disabled")
                    })?,
            )
        } else {
            // Coding/Domain campaign rows intentionally do not persist the
            // backend credential reference. On a later rerun, resolve the same
            // deterministic default the create UI uses instead of clearing
            // authProfiles and silently launching without a key.
            provider
                .auth_profiles
                .iter()
                .find(|profile| profile.enabled && !profile.api_key.trim().is_empty())
        };
        if let Some(profile) = selected_profile {
            if profile.api_key.trim().is_empty() {
                bail!("selected Provider credential profile has no API key");
            }
            provider.api_key.clone_from(&profile.api_key);
            if let Some(base_url) = &profile.base_url {
                provider.base_url.clone_from(base_url);
            }
        }
        validate_eval_base_url(&provider.base_url)?;
        provider.auth_profiles.clear();
        if let Some(existing) = resolved.get(&provider.id) {
            if ha_eval_spec::digest_serializable(existing)?
                != ha_eval_spec::digest_serializable(&provider)?
            {
                bail!("one campaign cannot select multiple credentials for one Provider");
            }
        } else {
            resolved.insert(provider.id.clone(), provider);
        }
    }
    Ok(resolved.into_values().collect())
}

pub fn list_model_options() -> Result<Vec<EvalModelOption>> {
    let config = crate::config::load_config()?;
    let codex_authenticated = crate::get_codex_token_cache()
        .and_then(|cache| cache.try_lock().ok().map(|value| value.is_some()))
        .unwrap_or(false)
        || crate::oauth::load_token().ok().flatten().is_some();
    let mut options = Vec::new();
    for provider in config.providers.iter().filter(|provider| provider.enabled) {
        let codex = provider.api_type.is_codex();
        let has_credential = !provider.api_key.trim().is_empty()
            || provider
                .auth_profiles
                .iter()
                .any(|profile| profile.enabled && !profile.api_key.trim().is_empty())
            || is_keyless_loopback(provider)?;
        for model in &provider.models {
            let cost_known = match (model.cost_input, model.cost_output) {
                (None, None) => false,
                (input, output) => [input, output]
                    .into_iter()
                    .flatten()
                    .all(|price| price.is_finite() && price >= 0.0),
            };
            let supports_isolated_eval = if codex {
                codex_authenticated
            } else {
                has_credential
            };
            let mut warnings = Vec::new();
            if codex && !codex_authenticated {
                warnings.push("codex_oauth_missing".to_string());
            } else if codex {
                warnings.push("codex_oauth_local_diagnostic_only".to_string());
            } else if !has_credential {
                warnings.push("provider_credential_missing".to_string());
            }
            if !cost_known {
                warnings.push("model_price_unknown".to_string());
            }
            options.push(EvalModelOption {
                provider_id: provider.id.clone(),
                model_id: model.id.clone(),
                label: model.name.clone(),
                provider_label: provider.name.clone(),
                credential_profile_label: provider
                    .auth_profiles
                    .iter()
                    .find(|profile| profile.enabled)
                    .map(|profile| profile.label.clone()),
                credential_profiles: provider
                    .auth_profiles
                    .iter()
                    .filter(|profile| profile.enabled && !profile.api_key.trim().is_empty())
                    .map(|profile| EvalCredentialOption {
                        credential_profile_ref: profile.id.clone(),
                        label: profile.label.clone(),
                    })
                    .collect(),
                supports_isolated_eval,
                cost_known,
                warnings,
            });
        }
    }
    options.sort_by(|left, right| {
        left.provider_label
            .cmp(&right.provider_label)
            .then_with(|| left.label.cmp(&right.label))
    });
    Ok(options)
}

#[allow(clippy::too_many_arguments)]
pub async fn resolve_local_launch(
    request: EvalAppRunRequest,
    reference: String,
    dirty: bool,
    app_version: String,
    product_binary_digest: String,
    runner_binary_digest: String,
    asset_root_digest: String,
) -> Result<EvalResolvedLaunch> {
    ha_eval_spec::app::validate_app_request(&request)?;
    let source = crate::config::load_config()?;
    let mut isolated = crate::config::AppConfig::default();
    let mut models = Vec::with_capacity(request.models.len());
    let mut secrets = BTreeMap::<String, String>::new();
    let mut credential_identity = BTreeMap::<String, String>::new();
    let mut resolved_codex_token: Option<crate::oauth::CodexEvaluationToken> = None;
    for selection in &request.models {
        let provider = source
            .providers
            .iter()
            .find(|provider| provider.id == selection.provider_id && provider.enabled)
            .ok_or_else(|| anyhow!("selected evaluation Provider is missing or disabled"))?;
        let model = provider
            .models
            .iter()
            .find(|model| model.id == selection.model_id)
            .ok_or_else(|| {
                anyhow!("selected evaluation model is not registered by its Provider")
            })?;
        let (secret, credential_metadata, base_url_override) = if provider.api_type.is_codex() {
            if selection.credential_profile_ref.is_some() {
                bail!("Codex evaluation models do not accept API-key credential profiles");
            }
            if resolved_codex_token.is_none() {
                let required_validity_secs = request
                    .campaign_budget
                    .max_wall_seconds
                    .ok_or_else(|| {
                        anyhow!(
                            "Codex evaluation requires maxWallSeconds because the isolated runtime cannot refresh OAuth"
                        )
                    })?;
                resolved_codex_token = Some(
                    crate::oauth::load_codex_token_for_evaluation(required_validity_secs).await?,
                );
            }
            let token = resolved_codex_token
                .as_ref()
                .ok_or_else(|| anyhow!("Codex OAuth credential is unavailable"))?;
            let secret = crate::config::encode_model_eval_codex_secret(
                &token.access_token,
                &token.account_id,
                token.expires_at_ms,
            )?;
            (
                Some(secret),
                serde_json::json!({
                    "kind": "codex_oauth_access_token",
                    "accountIdDigest": digest_serializable(&token.account_id)?,
                }),
                None,
            )
        } else {
            resolve_credential(provider, selection.credential_profile_ref.as_deref())?
        };
        let credential_digest = digest_serializable(&credential_metadata)?;
        if let Some(existing) =
            credential_identity.insert(provider.id.clone(), credential_digest.clone())
        {
            if existing != credential_digest {
                bail!("one App experiment cannot use different credential profiles for the same Provider");
            }
        }
        if let Some(secret) = secret {
            if let Some(existing) = secrets.insert(provider.id.clone(), secret.clone()) {
                if existing != secret {
                    bail!("evaluation Provider credential resolution is ambiguous");
                }
            }
        }
        let mut sanitized_provider = provider.clone();
        sanitized_provider.api_key.clear();
        sanitized_provider.auth_profiles.clear();
        if let Some(base_url) = base_url_override {
            sanitized_provider.base_url = base_url;
        }
        validate_eval_base_url(&sanitized_provider.base_url)?;
        if let Some(existing) = isolated
            .providers
            .iter()
            .find(|registered| registered.id == sanitized_provider.id)
        {
            if digest_serializable(existing)? != digest_serializable(&sanitized_provider)? {
                bail!("evaluation Provider has inconsistent per-model configuration");
            }
        } else {
            isolated.providers.push(sanitized_provider.clone());
        }
        let max_output_tokens = selection
            .max_output_tokens
            .or(Some(u64::from(model.max_tokens)))
            .filter(|value| *value > 0);
        let profile = ModelProfile {
            role: "anchor".to_string(),
            provider_id: provider.id.clone(),
            model_id: model.id.clone(),
            snapshot: Some(model.id.clone()),
            temperature: source.temperature,
            reasoning_effort: selection.reasoning_effort.clone().or_else(|| {
                (!source.reasoning_effort.trim().is_empty())
                    .then(|| source.reasoning_effort.clone())
            }),
            max_output_tokens,
        };
        models.push(AppResolvedModelBinding {
            model: profile,
            provider_config_digest: digest_serializable(&sanitized_provider)?,
            credential_config_digest: credential_digest,
        });
    }
    isolated.active_model = models.first().map(|binding| crate::provider::ActiveModel {
        provider_id: binding.model.provider_id.clone(),
        model_id: binding.model.model_id.clone(),
    });
    isolated.fallback_models.clear();
    isolated.channels = Default::default();
    isolated.cron = Default::default();
    isolated.memory = Default::default();
    isolated.memory_extract = Default::default();
    isolated.dreaming = Default::default();
    isolated.awareness = Default::default();
    isolated.mcp_servers.clear();
    isolated.disable_all_hooks = true;
    let credential_free_config = serde_json::to_value(isolated)?;
    ha_eval_spec::model::reject_embedded_secrets(&credential_free_config, "$.config")?;
    let provider_secrets_b64 = if secrets.is_empty() {
        String::new()
    } else {
        base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&secrets)?)
    };
    Ok(EvalResolvedLaunch {
        request,
        models,
        runtime_environment: RuntimeEnvironmentSnapshot {
            actual_runner_class: "local_native".to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            os_version: None,
            network_enforcement: NetworkEnforcement::Unverified,
            product_binary_digest,
            runner_binary_digest,
            asset_root_digest,
            hardware_class_digest: None,
            container_image_digest: None,
        },
        reference,
        dirty,
        app_version,
        credential_free_config,
        provider_secrets_b64,
    })
}

fn resolve_credential(
    provider: &crate::provider::ProviderConfig,
    requested_profile: Option<&str>,
) -> Result<(Option<String>, serde_json::Value, Option<String>)> {
    if let Some(profile_id) = requested_profile {
        let profile = provider
            .auth_profiles
            .iter()
            .find(|profile| profile.id == profile_id && profile.enabled)
            .ok_or_else(|| {
                anyhow!("selected Provider credential profile is missing or disabled")
            })?;
        if profile.api_key.trim().is_empty() {
            bail!("selected Provider credential profile has no API key");
        }
        return Ok((
            Some(profile.api_key.clone()),
            serde_json::json!({
                "kind": "auth_profile",
                "label": profile.label,
                "baseUrl": profile.base_url,
            }),
            profile.base_url.clone(),
        ));
    }
    if let Some(profile) = provider
        .auth_profiles
        .iter()
        .find(|profile| profile.enabled && !profile.api_key.trim().is_empty())
    {
        return Ok((
            Some(profile.api_key.clone()),
            serde_json::json!({
                "kind": "auth_profile",
                "label": profile.label,
                "baseUrl": profile.base_url,
            }),
            profile.base_url.clone(),
        ));
    }
    if !provider.api_key.trim().is_empty() {
        return Ok((
            Some(provider.api_key.clone()),
            serde_json::json!({"kind": "legacy_key", "baseUrl": provider.base_url}),
            None,
        ));
    }
    if is_keyless_loopback(provider)? {
        return Ok((
            None,
            serde_json::json!({"kind": "keyless_loopback", "baseUrl": provider.base_url}),
            None,
        ));
    }
    bail!("selected Provider has no usable isolated-evaluation credential")
}

fn is_keyless_loopback(provider: &crate::provider::ProviderConfig) -> Result<bool> {
    if !provider.api_key.trim().is_empty()
        || provider
            .auth_profiles
            .iter()
            .any(|profile| profile.enabled && !profile.api_key.trim().is_empty())
    {
        return Ok(false);
    }
    let url = validate_eval_base_url(&provider.base_url)?;
    Ok(is_loopback_host(&url))
}

fn validate_eval_base_url(value: &str) -> Result<url::Url> {
    let url = url::Url::parse(value).context("parsing evaluation Provider base URL")?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("evaluation Provider base URL may not contain credentials, query, or fragment");
    }
    let loopback = is_loopback_host(&url);
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        bail!("evaluation Provider base URL must use HTTPS or loopback HTTP");
    }
    Ok(url)
}

fn is_loopback_host(url: &url::Url) -> bool {
    matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluation_provider_urls_reject_embedded_credentials_and_plaintext_remote_hosts() {
        assert!(validate_eval_base_url("https://api.example.com/v1").is_ok());
        assert!(validate_eval_base_url("http://127.0.0.1:11434/v1").is_ok());
        assert!(validate_eval_base_url("http://api.example.com/v1").is_err());
        assert!(validate_eval_base_url("https://token@api.example.com/v1").is_err());
        assert!(validate_eval_base_url("https://api.example.com/v1?api_key=secret").is_err());
    }
}
