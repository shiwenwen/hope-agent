//! macOS desktop control bridge and readiness model.
//!
//! Phase 2A exposes status / permissions plus a read-only Accessibility
//! snapshot model. ScreenCaptureKit frames and mutating input actions are
//! intentionally left for later phases.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::permissions::{SystemPermissionItem, SystemPermissionStatus, SystemPermissionsResponse};

const REQUIRED_PERMISSION_IDS: &[&str] = &["accessibility", "screen_recording"];
const OPTIONAL_PERMISSION_IDS: &[&str] = &[
    "automation_system_events",
    "input_monitoring",
    "system_audio_capture",
];
const DEFAULT_SNAPSHOT_MAX_ELEMENTS: usize = 120;
const DEFAULT_SNAPSHOT_MAX_DEPTH: usize = 8;
const HARD_SNAPSHOT_MAX_ELEMENTS: usize = 500;
const HARD_SNAPSHOT_MAX_DEPTH: usize = 16;

#[async_trait]
pub trait MacControlBridge: Send + Sync {
    async fn system_permissions(&self) -> SystemPermissionsResponse;
    async fn snapshot(
        &self,
        request: MacControlSnapshotRequest,
    ) -> Result<MacControlSnapshot, String>;
}

static MAC_CONTROL_BRIDGE: OnceLock<Arc<dyn MacControlBridge>> = OnceLock::new();

pub fn set_mac_control_bridge(bridge: Arc<dyn MacControlBridge>) {
    let _ = MAC_CONTROL_BRIDGE.set(bridge);
}

pub fn get_mac_control_bridge() -> Option<Arc<dyn MacControlBridge>> {
    MAC_CONTROL_BRIDGE.get().cloned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlReadiness {
    Ready,
    Limited,
    Blocked,
    Unsupported,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlPermissionSummary {
    pub id: String,
    pub status: SystemPermissionStatus,
    pub required: bool,
    pub optional: bool,
    pub settings_pane: Option<String>,
    pub usage: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlStatus {
    pub platform: String,
    pub supported: bool,
    pub desktop: bool,
    pub bridge_registered: bool,
    pub readiness: MacControlReadiness,
    pub core_ready: bool,
    pub required_permissions: Vec<MacControlPermissionSummary>,
    pub optional_permissions: Vec<MacControlPermissionSummary>,
    pub missing_required: Vec<String>,
    pub optional_pending: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlPermissionsResponse {
    pub status: MacControlStatus,
    pub system_permissions: SystemPermissionsResponse,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlSnapshotRequest {
    #[serde(default)]
    pub include_screenshot: bool,
    #[serde(default = "default_snapshot_max_elements")]
    pub max_elements: usize,
    #[serde(default = "default_snapshot_max_depth")]
    pub max_depth: usize,
}

impl Default for MacControlSnapshotRequest {
    fn default() -> Self {
        Self {
            include_screenshot: false,
            max_elements: DEFAULT_SNAPSHOT_MAX_ELEMENTS,
            max_depth: DEFAULT_SNAPSHOT_MAX_DEPTH,
        }
    }
}

impl MacControlSnapshotRequest {
    pub fn clamped(mut self) -> Self {
        if self.max_elements == 0 {
            self.max_elements = DEFAULT_SNAPSHOT_MAX_ELEMENTS;
        }
        if self.max_depth == 0 {
            self.max_depth = DEFAULT_SNAPSHOT_MAX_DEPTH;
        }
        self.max_elements = self.max_elements.min(HARD_SNAPSHOT_MAX_ELEMENTS);
        self.max_depth = self.max_depth.min(HARD_SNAPSHOT_MAX_DEPTH);
        self
    }
}

fn default_snapshot_max_elements() -> usize {
    DEFAULT_SNAPSHOT_MAX_ELEMENTS
}

fn default_snapshot_max_depth() -> usize {
    DEFAULT_SNAPSHOT_MAX_DEPTH
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlSnapshotResponse {
    pub status: MacControlStatus,
    pub snapshot: Option<MacControlSnapshot>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlSnapshot {
    pub snapshot_id: String,
    pub created_at: String,
    pub frontmost_app: Option<MacControlAppSummary>,
    pub displays: Vec<MacControlDisplaySummary>,
    pub windows: Vec<MacControlWindowSummary>,
    pub elements: Vec<MacControlElementSummary>,
    pub screenshot: Option<MacControlScreenshotSummary>,
    pub truncated: bool,
    pub warnings: Vec<String>,
}

impl MacControlSnapshot {
    pub fn new_empty() -> Self {
        Self {
            snapshot_id: new_snapshot_id(),
            created_at: Utc::now().to_rfc3339(),
            frontmost_app: None,
            displays: Vec::new(),
            windows: Vec::new(),
            elements: Vec::new(),
            screenshot: None,
            truncated: false,
            warnings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlAppSummary {
    pub pid: i32,
    pub bundle_id: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlDisplaySummary {
    pub id: u32,
    pub frame_points: MacControlBounds,
    pub scale: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlWindowSummary {
    pub id: String,
    pub app_pid: Option<i32>,
    pub title: Option<String>,
    pub focused: bool,
    pub bounds_points: Option<MacControlBounds>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlElementSummary {
    pub id: String,
    pub window_id: Option<String>,
    pub role: Option<String>,
    pub label: Option<String>,
    pub value: Option<String>,
    pub enabled: Option<bool>,
    pub focused: bool,
    pub bounds_points: Option<MacControlBounds>,
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlScreenshotSummary {
    pub media_id: String,
    pub path: String,
    pub width_px: u32,
    pub height_px: u32,
}

pub async fn status() -> MacControlStatus {
    let Some(bridge) = available_bridge() else {
        return unsupported_status(unsupported_reason());
    };
    let permissions = bridge.system_permissions().await;
    status_from_system_permissions(true, true, permissions)
}

pub async fn permissions() -> MacControlPermissionsResponse {
    let Some(bridge) = available_bridge() else {
        return unsupported_permissions_response(unsupported_reason());
    };
    let system_permissions = bridge.system_permissions().await;
    let status = status_from_system_permissions(true, true, system_permissions.clone());
    MacControlPermissionsResponse {
        status,
        system_permissions,
    }
}

pub async fn snapshot(request: MacControlSnapshotRequest) -> MacControlSnapshotResponse {
    let Some(bridge) = available_bridge() else {
        return unsupported_snapshot_response(unsupported_reason());
    };
    let system_permissions = bridge.system_permissions().await;
    let status = status_from_system_permissions(true, true, system_permissions.clone());
    if !system_permissions.supported {
        return MacControlSnapshotResponse {
            status,
            snapshot: None,
            error: Some("macOS control is unsupported in this runtime.".to_string()),
        };
    }
    if !permission_granted(&system_permissions, "accessibility") {
        return MacControlSnapshotResponse {
            status,
            snapshot: None,
            error: Some("Snapshot requires Accessibility permission.".to_string()),
        };
    }
    if request.include_screenshot && !permission_granted(&system_permissions, "screen_recording") {
        return MacControlSnapshotResponse {
            status,
            snapshot: None,
            error: Some("Screenshot snapshots require Screen Recording permission.".to_string()),
        };
    }

    match bridge.snapshot(request.clamped()).await {
        Ok(snapshot) => MacControlSnapshotResponse {
            status,
            snapshot: Some(snapshot),
            error: None,
        },
        Err(error) => MacControlSnapshotResponse {
            status,
            snapshot: None,
            error: Some(error),
        },
    }
}

fn available_bridge() -> Option<Arc<dyn MacControlBridge>> {
    if !cfg!(target_os = "macos") || !crate::app_init::is_desktop() {
        return None;
    }
    get_mac_control_bridge()
}

fn unsupported_reason() -> &'static str {
    if !cfg!(target_os = "macos") {
        "macOS control is only supported on macOS."
    } else if !crate::app_init::is_desktop() {
        "macOS control is only available from the desktop app."
    } else {
        "macOS control bridge is not registered."
    }
}

pub fn unsupported_status(message: &str) -> MacControlStatus {
    MacControlStatus {
        platform: platform_name().to_string(),
        supported: false,
        desktop: crate::app_init::is_desktop(),
        bridge_registered: get_mac_control_bridge().is_some(),
        readiness: MacControlReadiness::Unsupported,
        core_ready: false,
        required_permissions: Vec::new(),
        optional_permissions: Vec::new(),
        missing_required: Vec::new(),
        optional_pending: Vec::new(),
        message: message.to_string(),
    }
}

pub fn unsupported_permissions_response(message: &str) -> MacControlPermissionsResponse {
    MacControlPermissionsResponse {
        status: unsupported_status(message),
        system_permissions: unsupported_system_permissions(),
    }
}

pub fn unsupported_snapshot_response(message: &str) -> MacControlSnapshotResponse {
    MacControlSnapshotResponse {
        status: unsupported_status(message),
        snapshot: None,
        error: Some(message.to_string()),
    }
}

fn unsupported_system_permissions() -> SystemPermissionsResponse {
    SystemPermissionsResponse {
        platform: platform_name().to_string(),
        supported: false,
        items: Vec::new(),
    }
}

fn platform_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    }
}

fn status_from_system_permissions(
    desktop: bool,
    bridge_registered: bool,
    response: SystemPermissionsResponse,
) -> MacControlStatus {
    if !response.supported {
        return MacControlStatus {
            platform: response.platform,
            supported: false,
            desktop,
            bridge_registered,
            readiness: MacControlReadiness::Unsupported,
            core_ready: false,
            required_permissions: Vec::new(),
            optional_permissions: Vec::new(),
            missing_required: Vec::new(),
            optional_pending: Vec::new(),
            message: "macOS control is unsupported in this runtime.".to_string(),
        };
    }

    let required_permissions = summaries_for(&response, REQUIRED_PERMISSION_IDS, true);
    let optional_permissions = summaries_for(&response, OPTIONAL_PERMISSION_IDS, false);
    let missing_required = required_permissions
        .iter()
        .filter(|item| item.status != SystemPermissionStatus::Granted)
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let optional_pending = optional_permissions
        .iter()
        .filter(|item| optional_status_needs_attention(item.status))
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let core_ready = missing_required.is_empty();
    let readiness = if !core_ready {
        MacControlReadiness::Blocked
    } else if !optional_pending.is_empty() {
        MacControlReadiness::Limited
    } else {
        MacControlReadiness::Ready
    };
    let message = match readiness {
        MacControlReadiness::Ready => "macOS control is ready.".to_string(),
        MacControlReadiness::Limited => {
            "Core macOS control is ready; optional permissions are still pending.".to_string()
        }
        MacControlReadiness::Blocked => {
            "macOS control needs Accessibility and Screen Recording permissions.".to_string()
        }
        MacControlReadiness::Unsupported => "macOS control is unsupported.".to_string(),
    };

    MacControlStatus {
        platform: response.platform,
        supported: true,
        desktop,
        bridge_registered,
        readiness,
        core_ready,
        required_permissions,
        optional_permissions,
        missing_required,
        optional_pending,
        message,
    }
}

fn summaries_for(
    response: &SystemPermissionsResponse,
    ids: &[&str],
    required: bool,
) -> Vec<MacControlPermissionSummary> {
    ids.iter()
        .map(|id| {
            response
                .items
                .iter()
                .find(|item| item.id == *id)
                .map(|item| summary_from_item(item, required))
                .unwrap_or_else(|| missing_summary(id, required))
        })
        .collect()
}

fn summary_from_item(item: &SystemPermissionItem, required: bool) -> MacControlPermissionSummary {
    MacControlPermissionSummary {
        id: item.id.clone(),
        status: item.status,
        required,
        optional: !required,
        settings_pane: item.settings_pane.clone(),
        usage: item.usage.clone(),
        note: item.note.clone(),
    }
}

fn missing_summary(id: &str, required: bool) -> MacControlPermissionSummary {
    MacControlPermissionSummary {
        id: id.to_string(),
        status: SystemPermissionStatus::NotApplicable,
        required,
        optional: !required,
        settings_pane: None,
        usage: "Permission status is unavailable in this runtime.".to_string(),
        note: Some("The system permissions catalog did not return this item.".to_string()),
    }
}

fn optional_status_needs_attention(status: SystemPermissionStatus) -> bool {
    !matches!(
        status,
        SystemPermissionStatus::Granted
            | SystemPermissionStatus::NotApplicable
            | SystemPermissionStatus::NotUsed
    )
}

fn permission_granted(response: &SystemPermissionsResponse, id: &str) -> bool {
    response
        .items
        .iter()
        .any(|item| item.id == id && item.status == SystemPermissionStatus::Granted)
}

fn new_snapshot_id() -> String {
    format!("macsnap_{}", Uuid::new_v4().simple())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::{SystemPermissionGroup, SystemPermissionRequestMode};

    fn item(id: &str, status: SystemPermissionStatus) -> SystemPermissionItem {
        SystemPermissionItem {
            id: id.to_string(),
            group: SystemPermissionGroup::ControlCapture,
            status,
            request_mode: SystemPermissionRequestMode::OpenSettings,
            settings_pane: None,
            usage: String::new(),
            note: None,
        }
    }

    fn response(items: Vec<SystemPermissionItem>) -> SystemPermissionsResponse {
        SystemPermissionsResponse {
            platform: "macos".to_string(),
            supported: true,
            items,
        }
    }

    #[test]
    fn readiness_is_ready_when_core_and_optional_permissions_are_granted() {
        let status = status_from_system_permissions(
            true,
            true,
            response(vec![
                item("accessibility", SystemPermissionStatus::Granted),
                item("screen_recording", SystemPermissionStatus::Granted),
                item("automation_system_events", SystemPermissionStatus::Granted),
                item("input_monitoring", SystemPermissionStatus::Granted),
                item("system_audio_capture", SystemPermissionStatus::Granted),
            ]),
        );

        assert_eq!(status.readiness, MacControlReadiness::Ready);
        assert!(status.core_ready);
        assert!(status.missing_required.is_empty());
        assert!(status.optional_pending.is_empty());
    }

    #[test]
    fn readiness_is_blocked_when_accessibility_is_missing() {
        let status = status_from_system_permissions(
            true,
            true,
            response(vec![
                item("accessibility", SystemPermissionStatus::NotGranted),
                item("screen_recording", SystemPermissionStatus::Granted),
            ]),
        );

        assert_eq!(status.readiness, MacControlReadiness::Blocked);
        assert!(!status.core_ready);
        assert_eq!(status.missing_required, vec!["accessibility"]);
    }

    #[test]
    fn readiness_is_blocked_when_screen_recording_is_missing() {
        let status = status_from_system_permissions(
            true,
            true,
            response(vec![
                item("accessibility", SystemPermissionStatus::Granted),
                item("screen_recording", SystemPermissionStatus::NotDetermined),
            ]),
        );

        assert_eq!(status.readiness, MacControlReadiness::Blocked);
        assert!(!status.core_ready);
        assert_eq!(status.missing_required, vec!["screen_recording"]);
    }

    #[test]
    fn readiness_is_limited_when_only_optional_permissions_are_pending() {
        let status = status_from_system_permissions(
            true,
            true,
            response(vec![
                item("accessibility", SystemPermissionStatus::Granted),
                item("screen_recording", SystemPermissionStatus::Granted),
                item(
                    "automation_system_events",
                    SystemPermissionStatus::ManualCheck,
                ),
            ]),
        );

        assert_eq!(status.readiness, MacControlReadiness::Limited);
        assert!(status.core_ready);
        assert_eq!(status.optional_pending, vec!["automation_system_events"]);
    }

    #[test]
    fn readiness_is_unsupported_when_system_permissions_are_unsupported() {
        let status = status_from_system_permissions(
            false,
            false,
            SystemPermissionsResponse {
                platform: "linux".to_string(),
                supported: false,
                items: Vec::new(),
            },
        );

        assert_eq!(status.readiness, MacControlReadiness::Unsupported);
        assert!(!status.supported);
        assert!(!status.core_ready);
        assert!(status.missing_required.is_empty());
    }

    #[test]
    fn snapshot_request_clamps_limits() {
        let request = MacControlSnapshotRequest {
            include_screenshot: true,
            max_elements: 10_000,
            max_depth: 100,
        }
        .clamped();

        assert!(request.include_screenshot);
        assert_eq!(request.max_elements, HARD_SNAPSHOT_MAX_ELEMENTS);
        assert_eq!(request.max_depth, HARD_SNAPSHOT_MAX_DEPTH);
    }

    #[test]
    fn unsupported_snapshot_response_has_consistent_shape() {
        let response = unsupported_snapshot_response("no desktop bridge");

        assert_eq!(response.status.readiness, MacControlReadiness::Unsupported);
        assert!(!response.status.supported);
        assert!(response.status.missing_required.is_empty());
        assert!(response.snapshot.is_none());
        assert_eq!(response.error.as_deref(), Some("no desktop bridge"));
    }
}
