//! Thin `#[tauri::command]` wrappers for ha-core functions.
//!
//! ha-core's business logic functions don't have `#[tauri::command]` attributes
//! (to stay Tauri-independent). This module provides the thin Tauri command layer.

// ── Permissions ──────────────────────────────────────────────────

#[tauri::command]
pub async fn check_all_permissions() -> ha_core::permissions::AllPermissions {
    ha_core::permissions::check_all_permissions().await
}

#[tauri::command]
pub async fn check_system_permissions() -> ha_core::permissions::SystemPermissionsResponse {
    ha_core::permissions::check_system_permissions().await
}

#[tauri::command]
pub async fn check_permission(id: String) -> ha_core::permissions::PermissionStatus {
    ha_core::permissions::check_permission(id).await
}

#[tauri::command]
pub async fn request_permission(id: String) -> ha_core::permissions::PermissionStatus {
    ha_core::permissions::request_permission(id).await
}

#[tauri::command]
pub async fn request_system_permission(id: String) -> ha_core::permissions::SystemPermissionItem {
    ha_core::permissions::request_system_permission(id).await
}

// ── macOS Control ────────────────────────────────────────────────

#[tauri::command]
pub async fn mac_control_status() -> ha_core::mac_control::MacControlStatus {
    ha_core::mac_control::status().await
}

#[tauri::command]
pub async fn mac_control_permissions() -> ha_core::mac_control::MacControlPermissionsResponse {
    ha_core::mac_control::permissions().await
}

#[tauri::command]
pub async fn mac_control_snapshot(
    options: Option<ha_core::mac_control::MacControlSnapshotRequest>,
) -> ha_core::mac_control::MacControlSnapshotResponse {
    ha_core::mac_control::snapshot(options.unwrap_or_default()).await
}

#[tauri::command]
pub async fn mac_control_elements(
    options: Option<ha_core::mac_control::MacControlElementsRequest>,
) -> ha_core::mac_control::MacControlElementsResponse {
    ha_core::mac_control::elements(options.unwrap_or_default()).await
}

#[tauri::command]
pub async fn mac_control_capture_frame() -> ha_core::mac_control::MacControlFrameResponse {
    ha_core::mac_control::capture_frame().await
}

// ── Sandbox ──────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_sandbox_config() -> Result<ha_core::sandbox::SandboxConfig, String> {
    ha_core::sandbox::get_sandbox_config().await
}

#[tauri::command]
pub async fn set_sandbox_config(config: ha_core::sandbox::SandboxConfig) -> Result<(), String> {
    ha_core::sandbox::set_sandbox_config(config).await
}

#[tauri::command]
pub async fn check_sandbox_available() -> ha_core::sandbox::DockerStatus {
    ha_core::sandbox::check_sandbox_available().await
}

// ── Slash Commands ───────────────────────────────────────────────
// ha-core's slash_commands read cross-runtime singletons (SessionDB,
// cached agent, etc.) via OnceLock accessors, so these wrappers are
// pure Tauri-surface adapters with no `State<AppState>` dependency.

#[tauri::command]
pub async fn list_slash_commands(
) -> Result<Vec<ha_core::slash_commands::types::SlashCommandDef>, String> {
    ha_core::slash_commands::list_slash_commands().await
}

#[tauri::command]
pub async fn execute_slash_command(
    session_id: Option<String>,
    agent_id: String,
    command_text: String,
) -> Result<ha_core::slash_commands::types::CommandResult, String> {
    ha_core::slash_commands::execute_slash_command(session_id, agent_id, command_text).await
}

#[tauri::command]
pub fn is_slash_command(text: String) -> bool {
    ha_core::slash_commands::is_slash_command(text)
}

// ── Canvas ───────────────────────────────────────────────────────

#[tauri::command]
pub async fn canvas_submit_snapshot(
    request_id: String,
    data_url: Option<String>,
    error: Option<String>,
) -> Result<(), String> {
    ha_core::tools::canvas::canvas_submit_snapshot(request_id, data_url, error).await
}

#[tauri::command]
pub async fn canvas_submit_eval_result(
    request_id: String,
    result: Option<String>,
    error: Option<String>,
) -> Result<(), String> {
    ha_core::tools::canvas::canvas_submit_eval_result(request_id, result, error).await
}

#[tauri::command]
pub async fn get_canvas_config() -> Result<ha_core::tools::canvas::CanvasConfig, String> {
    ha_core::tools::canvas::get_canvas_config().await
}

#[tauri::command]
pub async fn save_canvas_config(
    config: ha_core::tools::canvas::CanvasConfig,
) -> Result<(), String> {
    ha_core::tools::canvas::save_canvas_config(config).await
}

#[tauri::command]
pub async fn list_canvas_projects() -> Result<String, String> {
    ha_core::tools::canvas::list_canvas_projects().await
}

#[tauri::command]
pub async fn list_canvas_projects_by_session(
    session_id: String,
) -> Result<Vec<ha_core::tools::canvas::CanvasProjectView>, String> {
    ha_core::tools::canvas::list_canvas_projects_by_session(session_id).await
}

#[tauri::command]
pub async fn get_canvas_project(project_id: String) -> Result<String, String> {
    ha_core::tools::canvas::get_canvas_project(project_id).await
}

#[tauri::command]
pub async fn delete_canvas_project(project_id: String) -> Result<(), String> {
    ha_core::tools::canvas::delete_canvas_project(project_id).await
}

#[tauri::command]
pub async fn show_canvas_panel(project_id: String) -> Result<(), String> {
    ha_core::tools::canvas::show_canvas_panel(project_id).await
}

// ── Artifacts ────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactImportRequest {
    pub file_path: Option<String>,
    pub upload_id: Option<String>,
    pub artifact_id: Option<String>,
    pub expected_version: Option<i64>,
    pub title: Option<String>,
    pub kind: Option<String>,
    pub privacy: Option<String>,
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub agent_id: Option<String>,
    pub goal_id: Option<String>,
    pub version_message: Option<String>,
}

#[tauri::command]
pub async fn list_artifacts(
    limit: Option<usize>,
    offset: Option<usize>,
    kind: Option<String>,
    lifecycle_state: Option<String>,
) -> Result<Vec<crate::artifacts::ArtifactRecord>, String> {
    ha_core::blocking::run_blocking(move || {
        crate::artifacts::ArtifactService::open()
            .and_then(|service| {
                service.list(crate::artifacts::ListArtifactsInput {
                    limit: limit.unwrap_or(50),
                    offset: offset.unwrap_or(0),
                    kind,
                    lifecycle_state,
                })
            })
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub async fn get_artifact(id: String) -> Result<crate::artifacts::ArtifactRecord, String> {
    ha_core::blocking::run_blocking(move || {
        crate::artifacts::ArtifactService::open()
            .and_then(|service| service.get(&id))
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "artifact not found".to_string())
    })
    .await
}

#[tauri::command]
pub async fn list_artifact_versions(
    id: String,
) -> Result<Vec<crate::artifacts::ArtifactVersionSummary>, String> {
    ha_core::blocking::run_blocking(move || {
        crate::artifacts::ArtifactService::open()
            .and_then(|service| service.versions(&id))
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub async fn import_artifact(
    request: ArtifactImportRequest,
) -> Result<crate::artifacts::ArtifactRecord, String> {
    ha_core::blocking::run_blocking(move || {
        let source = match (request.file_path, request.upload_id) {
            (Some(file_path), None) if !file_path.trim().is_empty() => {
                let file_path = std::path::PathBuf::from(file_path);
                let allowed_roots = file_path
                    .canonicalize()
                    .ok()
                    .and_then(|path| path.parent().map(ToOwned::to_owned))
                    .into_iter()
                    .collect::<Vec<_>>();
                crate::artifacts::ArtifactImportSource::Path {
                    file_path,
                    allowed_roots: Some(allowed_roots),
                }
            }
            (None, Some(upload_id)) if !upload_id.trim().is_empty() => {
                crate::artifacts::ArtifactImportSource::Upload { upload_id }
            }
            _ => return Err("exactly one of filePath or uploadId is required".to_string()),
        };
        let producer = serde_json::json!({ "type": "owner_import", "surface": "tauri" });
        let mut service =
            crate::artifacts::ArtifactService::open().map_err(|error| error.to_string())?;
        if let Some(artifact_id) = request.artifact_id {
            let expected_version = request.expected_version.ok_or_else(|| {
                "expectedVersion is required when artifactId is provided".to_string()
            })?;
            let current = service
                .get(&artifact_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "artifact not found".to_string())?;
            if current.current_version != expected_version {
                return Err(format!(
                    "artifact version conflict: expected {}, current {} ({})",
                    expected_version, current.current_version, current.current_hash
                ));
            }
            service
                .update_from_source(crate::artifacts::UpdateArtifactInput {
                    artifact_id,
                    source,
                    expected_version,
                    title: request.title,
                    message: request.version_message,
                    producer,
                    incognito: false,
                })
                .map_err(|error| error.to_string())
        } else {
            service
                .create_from_source(crate::artifacts::CreateArtifactInput {
                    source,
                    title: request.title,
                    kind: crate::artifacts::ArtifactKind::parse(request.kind.as_deref()),
                    privacy: request
                        .privacy
                        .unwrap_or_else(|| "local_private".to_string()),
                    session_id: request.session_id,
                    project_id: request.project_id,
                    agent_id: request.agent_id,
                    goal_id: request.goal_id,
                    producer,
                    incognito: false,
                })
                .map_err(|error| error.to_string())
        }
    })
    .await
}

#[tauri::command]
pub async fn restore_artifact(
    id: String,
    version: i64,
) -> Result<crate::artifacts::ArtifactRecord, String> {
    ha_core::blocking::run_blocking(move || {
        crate::artifacts::ArtifactService::open()
            .and_then(|mut service| service.restore(&id, version))
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub async fn verify_artifact(id: String) -> Result<crate::artifacts::VerificationReport, String> {
    ha_core::blocking::run_blocking(move || {
        crate::artifacts::ArtifactService::open()
            .and_then(|service| service.verify(&id))
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub async fn review_artifact_export(
    id: String,
    audience: String,
    redaction_checked: bool,
) -> Result<ha_core::domain_workflow::DomainArtifactExportGuardReport, String> {
    ha_core::blocking::run_blocking(move || {
        crate::artifacts::ArtifactService::open()
            .and_then(|service| service.review_for_export(&id, &audience, redaction_checked))
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub async fn export_artifact(
    id: String,
    format: String,
    output_path: String,
) -> Result<crate::artifacts::ArtifactExportReceipt, String> {
    let runtime = tokio::runtime::Handle::current();
    ha_core::blocking::run_blocking(move || {
        let mut service =
            crate::artifacts::ArtifactService::open().map_err(|error| error.to_string())?;
        let receipt = runtime
            .block_on(service.export_async(&id, &format))
            .map_err(|error| error.to_string())?;
        if receipt.status != "ready" {
            return Ok(receipt);
        }
        let source_path = receipt
            .internal_path
            .as_deref()
            .ok_or_else(|| "artifact export file is missing".to_string())?;
        let bytes = std::fs::read(source_path).map_err(|error| error.to_string())?;
        crate::platform::write_atomic(std::path::Path::new(&output_path), &bytes)
            .map_err(|error| error.to_string())?;
        Ok(receipt)
    })
    .await
}

#[tauri::command]
pub async fn archive_artifact(id: String) -> Result<(), String> {
    ha_core::blocking::run_blocking(move || {
        crate::artifacts::ArtifactService::open()
            .and_then(|service| service.archive(&id))
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub async fn delete_artifact(id: String) -> Result<(), String> {
    ha_core::blocking::run_blocking(move || {
        crate::artifacts::ArtifactService::open()
            .and_then(|service| service.delete(&id))
            .map_err(|error| error.to_string())
    })
    .await
}

// ── Developer Tools ──────────────────────────────────────────────

#[tauri::command]
pub async fn dev_clear_sessions() -> Result<(), String> {
    ha_core::dev_tools::dev_clear_sessions().await
}

#[tauri::command]
pub async fn dev_clear_cron() -> Result<(), String> {
    ha_core::dev_tools::dev_clear_cron().await
}

#[tauri::command]
pub async fn dev_clear_memory() -> Result<(), String> {
    ha_core::dev_tools::dev_clear_memory().await
}

#[tauri::command]
pub async fn dev_reset_config() -> Result<(), String> {
    ha_core::dev_tools::dev_reset_config().await
}

#[tauri::command]
pub async fn dev_clear_all() -> Result<(), String> {
    ha_core::dev_tools::dev_clear_all().await
}
