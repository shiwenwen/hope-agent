/// Server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Address to bind the server to (e.g. "127.0.0.1:8420").
    pub bind_addr: String,
    /// Optional API key for authenticating requests.
    pub api_key: Option<String>,
    /// Allowed CORS origins. Empty = permissive (allow all).
    pub cors_origins: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:8420".to_string(),
            api_key: None,
            cors_origins: Vec::new(),
        }
    }
}
