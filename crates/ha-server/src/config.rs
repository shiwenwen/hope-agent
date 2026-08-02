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
    /// Allowed CORS origins. Empty = same-origin only.
    pub cors_origins: Vec<String>,
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
