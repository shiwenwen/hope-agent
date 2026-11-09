//! Explicit, read-only model-list discovery. Never probes inference or writes configuration.

use std::{collections::BTreeMap, time::Duration};

use serde::Serialize;
use serde_json::Value;

use super::{ApiType, ProviderConfig};

const MAX_BODY: usize = 2 * 1024 * 1024;
const MAX_MODELS: usize = 2000;
const MAX_PAGES: usize = 10;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredModel {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDiscovery {
    pub models: Vec<DiscoveredModel>,
    /// The bounded list is incomplete; users may still add models manually.
    pub truncated: bool,
}

fn list_url(config: &ProviderConfig) -> Result<url::Url, String> {
    if config.api_type == ApiType::Codex {
        return Err("Model discovery is unavailable for OAuth providers.".into());
    }
    let base = url::Url::parse(&config.base_url)
        .map_err(|_| "Enter a valid HTTP(S) API base URL.".to_string())?;
    if !matches!(base.scheme(), "http" | "https")
        || base.host_str().is_none()
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
        || crate::agent::is_complete_endpoint_url(&config.base_url)
    {
        return Err("Model discovery requires an HTTP(S) API base URL without credentials, query, fragment, or an inference endpoint path.".into());
    }
    url::Url::parse(&crate::agent::build_api_url(&config.base_url, "/v1/models"))
        .map_err(|_| "Invalid model-list endpoint.".into())
}

fn valid_label(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}

fn parse_page(value: &Value) -> Result<Vec<DiscoveredModel>, String> {
    let data = value["data"].as_array().ok_or_else(|| {
        "The endpoint did not return a supported model list (data array).".to_string()
    })?;
    Ok(data
        .iter()
        .filter_map(|item| {
            let id = item["id"].as_str()?.trim();
            if !valid_label(id) {
                return None;
            }
            let name = item["display_name"]
                .as_str()
                .or_else(|| item["name"].as_str())
                .filter(|name| valid_label(name))
                .unwrap_or(id);
            Some(DiscoveredModel {
                id: id.to_string(),
                name: name.to_string(),
            })
        })
        .collect())
}

/// Restore only request-local credentials, select the first enabled profile,
/// and preserve its endpoint and Anthropic workspace binding. No fallback or
/// key rotation is attempted when this explicit list request fails.
pub async fn discover_provider_models(config: ProviderConfig) -> Result<ModelDiscovery, String> {
    tokio::time::timeout(Duration::from_secs(30), discover_from_config(config))
        .await
        .map_err(|_| "Model discovery timed out. Try again explicitly.".to_string())?
}

async fn discover_from_config(mut config: ProviderConfig) -> Result<ModelDiscovery, String> {
    config.sanitize();
    if config.api_type == ApiType::Codex {
        return Err("Model discovery is unavailable for OAuth providers.".into());
    }
    let workspace = super::test::resolve_probe_profile(&mut config)?;
    let endpoint = list_url(&config)?;
    let cfg = crate::config::cached_config();
    let policy = if config.allow_private_network {
        crate::security::ssrf::SsrfPolicy::AllowPrivate
    } else {
        cfg.ssrf.default_policy
    };
    crate::security::ssrf::check_url(endpoint.as_str(), policy, &cfg.ssrf.trusted_hosts)
        .await
        .map_err(|_| "Model-list destination is blocked by the network policy.".to_string())?;
    let host = endpoint.host_str().expect("validated host");
    let addresses = crate::security::ssrf::resolve_checked_destination(
        host,
        endpoint.port_or_known_default().expect("HTTP port"),
        policy,
        &cfg.ssrf.trusted_hosts,
    )
    .await
    .map_err(|_| "Model-list destination is blocked or unavailable.".to_string())?;
    let client = super::apply_model_proxy(
        reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(&config.user_agent)
            .resolve_to_addrs(host, &addresses),
    )
    .build()
    .map_err(|_| "Unable to build the model-list client.".to_string())?;
    discover_with_client(&config, workspace.as_deref(), endpoint, &client).await
}

async fn discover_with_client(
    config: &ProviderConfig,
    workspace: Option<&str>,
    endpoint: url::Url,
    client: &reqwest::Client,
) -> Result<ModelDiscovery, String> {
    let mut models = BTreeMap::new();
    let mut cursor: Option<String> = None;
    let mut seen_cursors = std::collections::HashSet::new();
    for page in 0..MAX_PAGES {
        let mut url = endpoint.clone();
        if config.api_type == ApiType::Anthropic {
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", "100");
            if let Some(cursor) = &cursor {
                query.append_pair("after_id", cursor);
            }
        }
        let mut request = client.get(url);
        if config.api_type == ApiType::Anthropic {
            request = request.headers(
                super::anthropic_headers(&config.base_url, &config.api_key, workspace)
                    .map_err(|_| "Invalid model-list authentication headers.".to_string())?,
            );
        } else if !config.api_key.is_empty() {
            request = request.bearer_auth(&config.api_key);
        }
        let mut response = request.send().await.map_err(|_| {
            "Unable to read the model list. Check the endpoint and credentials.".to_string()
        })?;
        if !response.status().is_success() {
            // Never echo provider error bodies, URLs, keys or request headers.
            return Err(format!(
                "Model-list request returned HTTP {}. No inference probe was sent.",
                response.status().as_u16()
            ));
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_BODY as u64)
        {
            return Err("Model-list response exceeds the size limit.".into());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "Unable to read the model-list response.".to_string())?
        {
            if body.len() + chunk.len() > MAX_BODY {
                return Err("Model-list response exceeds the size limit.".into());
            }
            body.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&body)
            .map_err(|_| "The model-list response is not valid JSON.".to_string())?;
        for model in parse_page(&value)? {
            if !config.api_key.is_empty()
                && (model.id.contains(&config.api_key) || model.name.contains(&config.api_key))
            {
                return Err("The model list contains unexpected credential data.".into());
            }
            if models.len() == MAX_MODELS && !models.contains_key(&model.id) {
                return Ok(ModelDiscovery {
                    models: models.into_values().collect(),
                    truncated: true,
                });
            }
            models.entry(model.id.clone()).or_insert(model);
        }
        if value["has_more"].as_bool() != Some(true) {
            return Ok(ModelDiscovery {
                models: models.into_values().collect(),
                truncated: false,
            });
        }
        // Follow only the documented Anthropic cursor on this same endpoint.
        // Never follow a server-provided URL with credentials attached.
        let next = value["last_id"].as_str().filter(|id| valid_label(id));
        if !config.api_key.is_empty() && next.is_some_and(|id| id.contains(&config.api_key)) {
            return Err("The model list contains unexpected credential data.".into());
        }
        if config.api_type != ApiType::Anthropic
            || page + 1 == MAX_PAGES
            || next.is_none()
            || !seen_cursors.insert(next.unwrap().to_string())
        {
            return Ok(ModelDiscovery {
                models: models.into_values().collect(),
                truncated: true,
            });
        }
        cursor = next.map(str::to_string);
    }
    unreachable!("the last page returns an explicitly truncated result")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::{
        matchers::{header, method, path, query_param},
        Mock, MockServer, ResponseTemplate,
    };

    fn config(base: &str, api_type: ApiType) -> ProviderConfig {
        serde_json::from_value(json!({
            "id":"discovery-test", "name":"Synthetic", "apiType":api_type,
            "baseUrl":base, "apiKey":"synthetic-key", "authProfiles":[],
            "models":[], "enabled":true, "userAgent":"synthetic-discovery", "thinkingStyle":"none"
        }))
        .unwrap()
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn chat_and_responses_discovery_are_get_only_and_keep_case_sensitive_ids() {
        for api_type in [ApiType::OpenaiChat, ApiType::OpenaiResponses] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/v1/models"))
                .and(header("authorization", "Bearer synthetic-key"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[
                    {"id":"Model", "name":"One", "context_length":999999},
                    {"id":"model"}, {"id":"Model","name":"Duplicate"},
                    {"id":""}, {"id":3}, {"id":"bad\nname"}
                ]})))
                .expect(1)
                .mount(&server)
                .await;
            let cfg = config(&server.uri(), api_type);
            let result = discover_with_client(&cfg, None, list_url(&cfg).unwrap(), &client())
                .await
                .unwrap();
            assert_eq!(
                result
                    .models
                    .iter()
                    .map(|model| model.id.as_str())
                    .collect::<Vec<_>>(),
                ["Model", "model"]
            );
            assert_eq!(result.models[0].name, "One");
            assert!(!result.truncated);
            let serialized = serde_json::to_string(&result).unwrap();
            assert!(!serialized.contains("synthetic-key"));
            assert!(!serialized.contains("context_length"));
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn anthropic_paginates_only_fixed_endpoint_and_preserves_workspace_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(query_param("limit", "100"))
            .and(header("x-api-key", "synthetic-key"))
            .and(header("anthropic-workspace-id", "wrkspc_A"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data":[{"id":"a"}],"has_more":true,"last_id":"a"})),
            )
            .expect(1)
            .with_priority(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(query_param("after_id", "a"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"data":[{"id":"b","display_name":"Model B"}],"has_more":false}),
            ))
            .expect(1)
            .with_priority(1)
            .mount(&server)
            .await;
        // The helper enforces direct Anthropic workspace bindings. Use the
        // official base for header validation with an isolated local endpoint.
        let cfg = config("https://api.anthropic.com", ApiType::Anthropic);
        let endpoint = url::Url::parse(&format!("{}/v1/models", server.uri())).unwrap();
        let result = discover_with_client(&cfg, Some("wrkspc_A"), endpoint, &client())
            .await
            .unwrap();
        assert_eq!(result.models.len(), 2);
        assert_eq!(result.models[1].name, "Model B");
        assert!(!result.truncated);
    }

    #[tokio::test]
    async fn redirects_and_error_bodies_are_never_followed_or_echoed() {
        let server = MockServer::start().await;
        Mock::given(path("/v1/models"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/secret-target", server.uri()))
                    .set_body_string("synthetic-private-error"),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/secret-target"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let cfg = config(&server.uri(), ApiType::OpenaiChat);
        let error = discover_with_client(&cfg, None, list_url(&cfg).unwrap(), &client())
            .await
            .unwrap_err();
        assert!(error.contains("302"));
        assert!(!error.contains("synthetic-private-error"));
        assert!(!error.contains(&server.uri()));
    }

    #[tokio::test]
    async fn rejects_malformed_and_oversized_lists_without_inference_fallback() {
        for response in [
            ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"synthetic-key"}]})),
            ResponseTemplate::new(200).set_body_json(json!({"models":[]})),
            ResponseTemplate::new(200).set_body_string("<html>synthetic-private-error</html>"),
            ResponseTemplate::new(200).set_body_string("x".repeat(MAX_BODY + 1)),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/v1/models"))
                .respond_with(response)
                .expect(1)
                .mount(&server)
                .await;
            let cfg = config(&server.uri(), ApiType::OpenaiChat);
            let error = discover_with_client(&cfg, None, list_url(&cfg).unwrap(), &client())
                .await
                .unwrap_err();
            assert!(!error.contains("synthetic-private-error"));
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn reports_unsupported_pagination_and_model_limit_as_incomplete() {
        for body in [
            json!({"data":[{"id":"a"}],"has_more":true,"next":"http://untrusted.invalid"}),
            json!({"data":(0..=MAX_MODELS).map(|i| json!({"id":format!("model-{i}")})).collect::<Vec<_>>()}),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/v1/models"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .expect(1)
                .mount(&server)
                .await;
            let cfg = config(&server.uri(), ApiType::OpenaiChat);
            let result = discover_with_client(&cfg, None, list_url(&cfg).unwrap(), &client())
                .await
                .unwrap();
            assert!(result.truncated);
            assert!(result.models.len() <= MAX_MODELS);
        }
    }

    #[tokio::test]
    async fn never_puts_a_reflected_key_into_a_pagination_url() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data":[],"has_more":true,"last_id":"synthetic-key"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let cfg = config(&server.uri(), ApiType::Anthropic);
        let error = discover_with_client(&cfg, None, list_url(&cfg).unwrap(), &client())
            .await
            .unwrap_err();
        assert!(!error.contains("synthetic-key"));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(!requests[0].url.as_str().contains("synthetic-key"));
    }

    #[test]
    fn rejects_credential_urls_inference_paths_and_oauth_without_echoing_secrets() {
        for base in [
            "https://user:synthetic-key@example.com",
            "https://example.com?key=synthetic-key",
            "https://example.com/#fragment",
            "file:///tmp/test",
            "https://example.com/v1/chat/completions",
            "https://example.com/v1/responses",
            "https://example.com/v1/messages",
        ] {
            let error = list_url(&config(base, ApiType::OpenaiChat)).unwrap_err();
            assert!(!error.contains("synthetic-key"));
        }
        assert!(list_url(&config("https://example.com/v1", ApiType::Codex)).is_err());
        assert_eq!(
            list_url(&config("https://example.com/v1/", ApiType::OpenaiChat))
                .unwrap()
                .as_str(),
            "https://example.com/v1/models"
        );
    }
}
