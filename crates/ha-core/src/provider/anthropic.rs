use anyhow::Result;

pub const ANTHROPIC_BINDING_BETA: &str = "thinking-binding-controls-2026-08-01";

pub fn is_direct_anthropic(base_url: &str) -> bool {
    url::Url::parse(base_url).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("api.anthropic.com")
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
    })
}

/// A workspace binding is a credential-scoped owner choice, never guessed
/// from the API key or inherited from a different rotation profile.
pub fn anthropic_header_pairs<'a>(
    base_url: &str,
    api_key: &'a str,
    workspace_id: Option<&'a str>,
    binding_controls: bool,
) -> Result<Vec<(&'static str, &'a str)>> {
    let mut headers = vec![("x-api-key", api_key), ("anthropic-version", "2023-06-01")];
    if let Some(workspace_id) = workspace_id {
        let valid = is_direct_anthropic(base_url)
            && workspace_id.strip_prefix("wrkspc_").is_some_and(|suffix| {
                !suffix.is_empty()
                    && suffix.len() <= 128
                    && suffix.bytes().all(|byte| byte.is_ascii_alphanumeric())
            });
        if !valid {
            return Err(crate::failover::ProviderRequestContractError(
                "Anthropic multi-workspace keys require a valid workspace ID and the direct HTTPS endpoint. Configure the ID in this key's authentication profile.".to_string(),
            ).into());
        }
        headers.push(("anthropic-workspace-id", workspace_id));
    }
    if binding_controls && is_direct_anthropic(base_url) {
        headers.push(("anthropic-beta", ANTHROPIC_BINDING_BETA));
    }
    Ok(headers)
}

pub fn validate_anthropic_profiles(provider: &super::ProviderConfig) -> Result<()> {
    for profile in &provider.auth_profiles {
        if profile.anthropic_workspace_id.is_some() {
            if provider.api_type != super::ApiType::Anthropic {
                return Err(crate::failover::ProviderRequestContractError(
                    "Anthropic workspace binding requires the Anthropic API type.".to_string(),
                )
                .into());
            }
            anthropic_header_pairs(
                provider.resolve_base_url(profile),
                "",
                profile.anthropic_workspace_id.as_deref(),
                false,
            )?;
        }
    }
    Ok(())
}

pub fn anthropic_headers(
    base_url: &str,
    api_key: &str,
    workspace_id: Option<&str>,
) -> Result<reqwest::header::HeaderMap> {
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in
        crate::provider::anthropic_header_pairs(base_url, api_key, workspace_id, true)?
    {
        let mut value = reqwest::header::HeaderValue::from_str(value).map_err(|_| {
            crate::failover::ProviderRequestContractError(
                "Invalid Anthropic request header.".to_string(),
            )
        })?;
        value.set_sensitive(name == "x-api-key" || name == "anthropic-workspace-id");
        headers.insert(name, value);
    }
    Ok(headers)
}

/// Some compatible gateways accept only Bearer authentication. Replace the
/// authentication scheme while retaining the validated version/workspace headers.
pub fn anthropic_bearer_headers(
    base_url: &str,
    api_key: &str,
    workspace_id: Option<&str>,
) -> Result<reqwest::header::HeaderMap> {
    let mut headers = anthropic_headers(base_url, api_key, workspace_id)?;
    headers.remove("x-api-key");
    let mut authorization = reqwest::header::HeaderValue::from_str(&format!("Bearer {api_key}"))
        .map_err(|_| {
            crate::failover::ProviderRequestContractError(
                "Invalid Anthropic request header.".to_string(),
            )
        })?;
    authorization.set_sensitive(true);
    headers.insert(reqwest::header::AUTHORIZATION, authorization);
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_fallback_replaces_the_api_key_header_and_preserves_binding() {
        for (base_url, workspace) in [
            ("https://relay.example", None),
            ("https://api.anthropic.com", Some("wrkspc_A")),
        ] {
            let headers = anthropic_bearer_headers(base_url, "synthetic-key", workspace).unwrap();
            assert!(!headers.contains_key("x-api-key"));
            assert_eq!(headers["authorization"], "Bearer synthetic-key");
            assert!(headers["authorization"].is_sensitive());
            assert_eq!(headers["anthropic-version"], "2023-06-01");
            assert_eq!(
                headers
                    .get("anthropic-workspace-id")
                    .map(|id| id.to_str().unwrap()),
                workspace
            );
            assert!(!format!("{headers:?}").contains("synthetic-key"));
        }
    }

    #[test]
    fn workspace_binding_is_explicit_sensitive_and_direct_only() {
        let legacy = anthropic_headers("https://api.anthropic.com", "synthetic-key", None).unwrap();
        assert!(!legacy.contains_key("anthropic-workspace-id"));
        assert_eq!(legacy["anthropic-beta"], ANTHROPIC_BINDING_BETA);
        let bound = anthropic_headers(
            "https://api.anthropic.com/v1/messages",
            "synthetic-key",
            Some("wrkspc_01AbCd"),
        )
        .unwrap();
        assert_eq!(bound["anthropic-workspace-id"], "wrkspc_01AbCd");
        assert!(bound["anthropic-workspace-id"].is_sensitive());
        assert!(bound["x-api-key"].is_sensitive());
        assert!(!format!("{bound:?}").contains("synthetic-key"));
        assert!(!format!("{bound:?}").contains("wrkspc_01AbCd"));
        for base in [
            "https://relay.example",
            "http://api.anthropic.com",
            "https://api.anthropic.com.evil.example",
            "https://api.anthropic.com:8443",
        ] {
            assert!(anthropic_headers(base, "synthetic-key", Some("wrkspc_01AbCd")).is_err());
            let relay = anthropic_headers(base, "synthetic-key", None).unwrap();
            assert!(!relay.contains_key("anthropic-beta"));
        }
        for workspace in ["", " ", "workspace", "wrkspc_", "wrkspc_01\nsecret"] {
            let error = anthropic_headers(
                "https://api.anthropic.com",
                "synthetic-key",
                Some(workspace),
            )
            .unwrap_err();
            assert!(error
                .downcast_ref::<crate::failover::ProviderRequestContractError>()
                .is_some());
            assert!(!error.to_string().contains("secret"));
        }
    }

    #[test]
    fn provider_save_rejects_incomplete_or_misrouted_workspace_bindings() {
        let mut provider = super::super::ProviderConfig::new(
            "test".into(),
            super::super::ApiType::Anthropic,
            "https://api.anthropic.com".into(),
            String::new(),
        );
        let mut profile =
            super::super::AuthProfile::new("test".into(), "synthetic-key".into(), None);
        profile.anthropic_workspace_id = Some(String::new());
        provider.auth_profiles.push(profile);
        assert!(validate_anthropic_profiles(&provider).is_err());
        provider.auth_profiles[0].anthropic_workspace_id = Some("wrkspc_A".into());
        assert!(validate_anthropic_profiles(&provider).is_ok());
        provider.api_type = super::super::ApiType::OpenaiChat;
        assert!(validate_anthropic_profiles(&provider).is_err());
    }
}
