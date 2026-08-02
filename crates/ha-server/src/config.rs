use std::collections::HashSet;

use axum::http::Uri;

const CORS_ORIGINS_ENV: &str = "HA_CORS_ORIGINS";

/// Origins used by packaged Tauri webviews when they call an embedded or
/// remote Hope Agent server. They are explicit rather than wildcarded so an
/// arbitrary browser origin never inherits owner-session access.
const DESKTOP_CORS_ORIGINS: [&str; 2] = ["tauri://localhost", "http://tauri.localhost"];

/// Server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Address to bind the server to (e.g. "127.0.0.1:8420").
    pub bind_addr: String,
    /// Optional single Owner Token for authenticating requests.
    pub api_key: Option<String>,
    /// True when HA_API_KEY / HA_API_KEY_FILE owns rotation lifecycle.
    pub auth_externally_managed: bool,
    /// Optional token limited to read-only Knowledge Agent endpoints.
    pub knowledge_agent_read_token: Option<String>,
    /// Additional allowed CORS origins. Packaged desktop origins and
    /// `HA_CORS_ORIGINS` are merged at router construction time.
    pub cors_origins: Vec<String>,
}

/// Merge the packaged desktop origins, explicitly configured origins, and the
/// comma-separated deployment allowlist. Invalid values are ignored instead
/// of being passed to tower-http as malformed header values.
pub(crate) fn effective_cors_origins(configured: &[String]) -> Vec<String> {
    let from_env = std::env::var(CORS_ORIGINS_ENV).unwrap_or_default();
    effective_cors_origins_with_env(configured, &from_env)
}

fn effective_cors_origins_with_env(configured: &[String], from_env: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    DESKTOP_CORS_ORIGINS
        .iter()
        .copied()
        .chain(configured.iter().map(String::as_str))
        .chain(from_env.split(','))
        .filter_map(normalize_origin)
        .filter(|origin| seen.insert(origin.clone()))
        .collect()
}

fn normalize_origin(value: &str) -> Option<String> {
    let value = value.trim().trim_end_matches('/');
    let uri = value.parse::<Uri>().ok()?;
    let scheme = uri.scheme_str()?;
    let authority = uri.authority()?.as_str();
    if uri
        .path_and_query()
        .is_some_and(|part| part.as_str() != "/")
    {
        return None;
    }
    Some(format!("{scheme}://{authority}"))
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:8420".to_string(),
            api_key: None,
            auth_externally_managed: false,
            knowledge_agent_read_token: None,
            cors_origins: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cors_origins_merge_desktop_config_and_environment_allowlists() {
        let origins = effective_cors_origins_with_env(
            &[
                "https://configured.example/".to_string(),
                "not an origin".to_string(),
            ],
            " https://browser.example,https://configured.example/path ",
        );

        assert_eq!(
            origins,
            vec![
                "tauri://localhost",
                "http://tauri.localhost",
                "https://configured.example",
                "https://browser.example",
            ]
        );
    }
}
