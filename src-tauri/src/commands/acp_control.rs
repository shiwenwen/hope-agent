//! Tauri commands for ACP control plane management.

use crate::acp_control::config::AcpControlConfig;
use crate::acp_control::types::{AcpBackendInfo, AcpRun};
use crate::commands::CmdError;

/// List all registered ACP backends with their health status.
#[tauri::command]
pub async fn acp_list_backends() -> Result<Vec<AcpBackendInfo>, CmdError> {
    let store = ha_core::config::load_config()?;
    if !store.acp_control.enabled {
        return Ok(Vec::new());
    }

    let mut backends = Vec::new();
    for b in &store.acp_control.backends {
        let binary_path = if std::path::Path::new(&b.binary).is_absolute() {
            if std::path::Path::new(&b.binary).exists() {
                Some(b.binary.clone())
            } else {
                None
            }
        } else {
            crate::acp_control::registry::resolve_binary(&b.binary)
        };

        let health = if let Some(path) = &binary_path {
            crate::acp_control::health::probe_binary(path).await
        } else {
            crate::acp_control::health::build_health_status(
                false,
                None,
                None,
                Some(format!("Binary '{}' not found in PATH", b.binary)),
            )
        };

        backends.push(AcpBackendInfo {
            id: b.id.clone(),
            name: b.name.clone(),
            enabled: b.enabled,
            health,
            capabilities: crate::acp_control::types::AcpRuntimeCapabilities::default(),
        });
    }

    Ok(backends)
}

/// Run health checks on all backends.
#[tauri::command]
pub async fn acp_health_check() -> Result<Vec<AcpBackendInfo>, CmdError> {
    acp_list_backends().await
}

/// Refresh backend discovery (re-scan $PATH).
#[tauri::command]
pub async fn acp_refresh_backends() -> Result<(), CmdError> {
    // Re-discovery happens via registry if manager is initialized
    if let Some(manager) = crate::get_acp_manager() {
        let store = ha_core::config::load_config()?;
        let registry = std::sync::Arc::new(crate::acp_control::AcpRuntimeRegistry::new());
        crate::acp_control::registry::auto_discover_and_register(&registry, &store.acp_control)
            .await;
        let _ = manager; // Manager uses separate registry instance for now
    }
    Ok(())
}

/// List ACP runs for a parent session.
#[tauri::command]
pub async fn acp_list_runs(parent_session_id: Option<String>) -> Result<Vec<AcpRun>, CmdError> {
    if let Some(manager) = crate::get_acp_manager() {
        Ok(manager.list_runs(parent_session_id.as_deref()).await)
    } else if let Some(db) = crate::get_session_db() {
        // Fallback to DB
        if let Some(pid) = parent_session_id {
            db.list_acp_runs(&pid).map_err(Into::into)
        } else {
            Ok(Vec::new())
        }
    } else {
        Ok(Vec::new())
    }
}

/// Kill a specific ACP run.
#[tauri::command]
pub async fn acp_kill_run(run_id: String) -> Result<(), CmdError> {
    let manager = crate::get_acp_manager()
        .ok_or_else(|| CmdError::msg("ACP control plane not initialized"))?;
    manager.kill_run(&run_id).await.map_err(Into::into)
}

/// Get the full result of an ACP run.
#[tauri::command]
pub async fn acp_get_run_result(run_id: String) -> Result<String, CmdError> {
    let manager = crate::get_acp_manager()
        .ok_or_else(|| CmdError::msg("ACP control plane not initialized"))?;
    manager.get_result(&run_id).await.map_err(Into::into)
}

/// Get ACP control config.
#[tauri::command]
pub async fn acp_get_config() -> Result<AcpControlConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.acp_control)
}

/// Save ACP control config.
#[tauri::command]
pub async fn acp_set_config(config: AcpControlConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("acp_control", "settings-ui"), |store| {
        store.acp_control = config;
        Ok(())
    })
    .map_err(Into::into)
}
