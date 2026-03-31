//! ACP Control Plane — Runtime registry and auto-discovery.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::config::AcpControlConfig;
use super::types::{AcpBackendInfo, AcpHealthStatus, AcpRuntime};

/// Global registry of ACP runtime backends.
pub struct AcpRuntimeRegistry {
    backends: RwLock<HashMap<String, Arc<dyn AcpRuntime>>>,
    health_cache: RwLock<HashMap<String, AcpHealthStatus>>,
}

impl AcpRuntimeRegistry {
    pub fn new() -> Self {
        Self {
            backends: RwLock::new(HashMap::new()),
            health_cache: RwLock::new(HashMap::new()),
        }
    }

    /// Register a runtime backend.  Overwrites any existing backend with the same ID.
    pub async fn register(&self, runtime: Arc<dyn AcpRuntime>) {
        let id = runtime.backend_id().to_lowercase();
        self.backends.write().await.insert(id, runtime);
    }

    /// Unregister a runtime backend by ID.
    #[allow(dead_code)]
    pub async fn unregister(&self, backend_id: &str) {
        let id = backend_id.to_lowercase();
        self.backends.write().await.remove(&id);
        self.health_cache.write().await.remove(&id);
    }

    /// Get a specific backend by ID.
    pub async fn get(&self, backend_id: &str) -> Option<Arc<dyn AcpRuntime>> {
        self.backends
            .read()
            .await
            .get(&backend_id.to_lowercase())
            .cloned()
    }

    /// Get a backend, falling back to the first available one if `backend_id` is empty.
    pub async fn get_or_first_available(
        &self,
        backend_id: Option<&str>,
    ) -> Option<Arc<dyn AcpRuntime>> {
        if let Some(id) = backend_id {
            if !id.is_empty() {
                return self.get(id).await;
            }
        }
        // Return the first available backend
        let backends = self.backends.read().await;
        for runtime in backends.values() {
            if runtime.is_available().await {
                return Some(Arc::clone(runtime));
            }
        }
        backends.values().next().cloned()
    }

    /// List all registered backend IDs.
    pub async fn list_ids(&self) -> Vec<String> {
        self.backends.read().await.keys().cloned().collect()
    }

    /// Run health checks on all backends and return results.
    pub async fn health_check_all(&self) -> Vec<(String, AcpHealthStatus)> {
        let backends: Vec<(String, Arc<dyn AcpRuntime>)> = {
            self.backends
                .read()
                .await
                .iter()
                .map(|(k, v)| (k.clone(), Arc::clone(v)))
                .collect()
        };

        let mut results = Vec::with_capacity(backends.len());
        for (id, runtime) in backends {
            let status = runtime.health_check().await;
            self.health_cache
                .write()
                .await
                .insert(id.clone(), status.clone());
            results.push((id, status));
        }
        results
    }

    /// Get the cached health status for a backend.
    pub async fn cached_health(&self, backend_id: &str) -> Option<AcpHealthStatus> {
        self.health_cache
            .read()
            .await
            .get(&backend_id.to_lowercase())
            .cloned()
    }

    /// Build a list of `AcpBackendInfo` for the frontend.
    pub async fn list_backend_info(&self) -> Vec<AcpBackendInfo> {
        let backends = self.backends.read().await;
        let health_cache = self.health_cache.read().await;

        let mut infos = Vec::with_capacity(backends.len());
        for (id, runtime) in backends.iter() {
            let health = health_cache.get(id).cloned().unwrap_or(AcpHealthStatus {
                available: false,
                binary_path: None,
                version: None,
                error: Some("Not checked yet".into()),
                last_checked: chrono::Utc::now().to_rfc3339(),
            });

            infos.push(AcpBackendInfo {
                id: id.clone(),
                name: runtime.display_name().to_string(),
                enabled: true,
                health,
                capabilities: runtime.capabilities(),
            });
        }
        infos
    }

    /// Count how many backends are registered.
    pub async fn count(&self) -> usize {
        self.backends.read().await.len()
    }
}

// ── Auto-discovery ───────────────────────────────────────────────

/// Well-known ACP-compatible binaries to search for in $PATH.
const KNOWN_BINARIES: &[(&str, &str, &str)] = &[
    ("claude-code", "Claude Code", "claude"),
    ("codex-cli", "Codex CLI", "codex"),
    ("gemini-cli", "Gemini CLI", "gemini"),
];

/// Resolve a binary name to its full path using `which`.
pub fn resolve_binary(name: &str) -> Option<String> {
    which::which(name)
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

/// Auto-discover ACP backends from $PATH and from config, then register them.
pub async fn auto_discover_and_register(registry: &AcpRuntimeRegistry, config: &AcpControlConfig) {
    use super::runtime_stdio::StdioAcpRuntime;

    // 1. Register from user config
    for backend in &config.backends {
        if !backend.enabled {
            continue;
        }
        let binary_path = if std::path::Path::new(&backend.binary).is_absolute() {
            if std::path::Path::new(&backend.binary).exists() {
                Some(backend.binary.clone())
            } else {
                None
            }
        } else {
            resolve_binary(&backend.binary)
        };

        if let Some(path) = binary_path {
            let runtime = StdioAcpRuntime::new(
                backend.id.clone(),
                backend.name.clone(),
                path,
                backend.acp_args.clone(),
                backend.env.clone(),
            );
            registry.register(Arc::new(runtime)).await;
        }
    }

    // 2. Auto-discover known binaries not yet registered
    if config.auto_discover {
        let registered = registry.list_ids().await;
        for (id, name, binary) in KNOWN_BINARIES {
            if registered.iter().any(|r| r.eq_ignore_ascii_case(id)) {
                continue;
            }
            if let Some(path) = resolve_binary(binary) {
                let runtime = StdioAcpRuntime::new(
                    id.to_string(),
                    name.to_string(),
                    path,
                    vec![],
                    HashMap::new(),
                );
                registry.register(Arc::new(runtime)).await;
            }
        }
    }
}
