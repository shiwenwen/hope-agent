//! macOS desktop control bridge and readiness model.
//!
//! Phase 3A exposes status / permissions, read-only Accessibility snapshots,
//! primary-display screenshot frames, read-only wait/target matching, and
//! low-risk running-app focus control. Pointer/keyboard/window mutations are
//! intentionally left for later phases.

use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime},
};

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
const MAX_SNAPSHOT_CACHE: usize = 20;
const MAX_SCREENSHOT_FILES: usize = 100;
const DEFAULT_WAIT_TIMEOUT_MS: u64 = 10_000;
const DEFAULT_WAIT_POLL_MS: u64 = 500;
const HARD_WAIT_TIMEOUT_MS: u64 = 60_000;
const MIN_WAIT_POLL_MS: u64 = 100;
const HARD_WAIT_POLL_MS: u64 = 5_000;
pub const EVENT_MAC_CONTROL_FRAME: &str = "mac_control:frame";

#[async_trait]
pub trait MacControlBridge: Send + Sync {
    async fn system_permissions(&self) -> SystemPermissionsResponse;
    async fn snapshot(
        &self,
        request: MacControlSnapshotRequest,
    ) -> Result<MacControlSnapshot, String>;
    async fn capture_frame(&self) -> Result<MacControlFramePayload, String>;
    async fn apps(&self, request: MacControlAppsRequest) -> Result<MacControlAppsResult, String>;
}

static MAC_CONTROL_BRIDGE: OnceLock<Arc<dyn MacControlBridge>> = OnceLock::new();
static SNAPSHOT_CACHE: OnceLock<Mutex<VecDeque<MacControlSnapshot>>> = OnceLock::new();

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
pub struct MacControlWaitResponse {
    pub status: MacControlStatus,
    pub matched: bool,
    pub elapsed_ms: u64,
    pub attempts: u32,
    pub target: MacControlTargetQuery,
    pub matches: MacControlTargetMatches,
    pub snapshot: Option<MacControlSnapshot>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlFrameResponse {
    pub status: MacControlStatus,
    pub frame: Option<MacControlFramePayload>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlAppsResponse {
    pub status: MacControlStatus,
    pub result: Option<MacControlAppsResult>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlAppsResult {
    pub op: MacControlAppsOp,
    pub frontmost: Option<MacControlRunningApp>,
    pub apps: Vec<MacControlRunningApp>,
    pub activated: Option<MacControlRunningApp>,
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
pub struct MacControlRunningApp {
    pub pid: i32,
    pub bundle_id: Option<String>,
    pub name: Option<String>,
    pub active: bool,
    pub hidden: bool,
    pub activation_policy: String,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlFramePayload {
    pub snapshot_id: String,
    pub media_id: Option<String>,
    pub path: Option<String>,
    pub jpeg_base64: String,
    pub width_px: u32,
    pub height_px: u32,
    pub captured_at: i64,
    pub frontmost_app: Option<MacControlAppSummary>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlAppsRequest {
    #[serde(default)]
    pub op: MacControlAppsOp,
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default)]
    pub bundle_id: Option<String>,
    #[serde(default)]
    pub pid: Option<i32>,
    #[serde(default = "default_apps_limit")]
    pub limit: usize,
}

impl MacControlAppsRequest {
    pub fn clamped(mut self) -> Self {
        if self.limit == 0 {
            self.limit = default_apps_limit();
        }
        self.limit = self.limit.min(100);
        self
    }
}

fn default_apps_limit() -> usize {
    50
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlAppsOp {
    List,
    #[default]
    Frontmost,
    Activate,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlWaitRequest {
    #[serde(default)]
    pub target: MacControlTargetQuery,
    #[serde(default = "default_wait_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_wait_poll_ms")]
    pub poll_ms: u64,
    #[serde(default = "default_snapshot_max_elements")]
    pub max_elements: usize,
    #[serde(default = "default_snapshot_max_depth")]
    pub max_depth: usize,
}

impl MacControlWaitRequest {
    pub fn clamped(mut self) -> Self {
        if self.timeout_ms == 0 {
            self.timeout_ms = DEFAULT_WAIT_TIMEOUT_MS;
        }
        if self.poll_ms == 0 {
            self.poll_ms = DEFAULT_WAIT_POLL_MS;
        }
        if self.max_elements == 0 {
            self.max_elements = DEFAULT_SNAPSHOT_MAX_ELEMENTS;
        }
        if self.max_depth == 0 {
            self.max_depth = DEFAULT_SNAPSHOT_MAX_DEPTH;
        }
        self.timeout_ms = self.timeout_ms.min(HARD_WAIT_TIMEOUT_MS);
        self.poll_ms = self.poll_ms.clamp(MIN_WAIT_POLL_MS, HARD_WAIT_POLL_MS);
        self.max_elements = self.max_elements.min(HARD_SNAPSHOT_MAX_ELEMENTS);
        self.max_depth = self.max_depth.min(HARD_SNAPSHOT_MAX_DEPTH);
        self
    }
}

fn default_wait_timeout_ms() -> u64 {
    DEFAULT_WAIT_TIMEOUT_MS
}

fn default_wait_poll_ms() -> u64 {
    DEFAULT_WAIT_POLL_MS
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlTargetQuery {
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default)]
    pub bundle_id: Option<String>,
    #[serde(default)]
    pub window_title: Option<String>,
    #[serde(default)]
    pub element_id: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub focused: Option<bool>,
}

impl MacControlTargetQuery {
    fn is_empty(&self) -> bool {
        self.app_name.as_deref().is_none_or(str::is_empty)
            && self.bundle_id.as_deref().is_none_or(str::is_empty)
            && self.window_title.as_deref().is_none_or(str::is_empty)
            && self.element_id.as_deref().is_none_or(str::is_empty)
            && self.text.as_deref().is_none_or(str::is_empty)
            && self.role.as_deref().is_none_or(str::is_empty)
            && self.enabled.is_none()
            && self.focused.is_none()
    }

    fn wants_element(&self) -> bool {
        self.element_id
            .as_deref()
            .is_some_and(|value| !value.is_empty())
            || self.text.as_deref().is_some_and(|value| !value.is_empty())
            || self.role.as_deref().is_some_and(|value| !value.is_empty())
            || self.enabled.is_some()
            || self.focused.is_some()
    }

    fn wants_app(&self) -> bool {
        self.app_name
            .as_deref()
            .is_some_and(|value| !value.is_empty())
            || self
                .bundle_id
                .as_deref()
                .is_some_and(|value| !value.is_empty())
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlTargetMatches {
    pub app: Option<MacControlAppSummary>,
    pub windows: Vec<MacControlWindowSummary>,
    pub elements: Vec<MacControlElementSummary>,
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
        Ok(snapshot) => {
            record_snapshot(snapshot.clone());
            MacControlSnapshotResponse {
                status,
                snapshot: Some(snapshot),
                error: None,
            }
        }
        Err(error) => MacControlSnapshotResponse {
            status,
            snapshot: None,
            error: Some(error),
        },
    }
}

pub async fn wait(request: MacControlWaitRequest) -> MacControlWaitResponse {
    let request = request.clamped();
    let target = request.target.clone();
    let Some(bridge) = available_bridge() else {
        return unsupported_wait_response(unsupported_reason(), target);
    };
    let system_permissions = bridge.system_permissions().await;
    let status = status_from_system_permissions(true, true, system_permissions.clone());
    if target.is_empty() {
        return MacControlWaitResponse {
            status,
            matched: false,
            elapsed_ms: 0,
            attempts: 0,
            target,
            matches: MacControlTargetMatches::default(),
            snapshot: None,
            error: Some("mac_control wait requires at least one target field.".to_string()),
        };
    }
    if !system_permissions.supported {
        return MacControlWaitResponse {
            status,
            matched: false,
            elapsed_ms: 0,
            attempts: 0,
            target,
            matches: MacControlTargetMatches::default(),
            snapshot: None,
            error: Some("macOS control is unsupported in this runtime.".to_string()),
        };
    }
    if !permission_granted(&system_permissions, "accessibility") {
        return MacControlWaitResponse {
            status,
            matched: false,
            elapsed_ms: 0,
            attempts: 0,
            target,
            matches: MacControlTargetMatches::default(),
            snapshot: None,
            error: Some("mac_control wait requires Accessibility permission.".to_string()),
        };
    }

    let started = Instant::now();
    let timeout = Duration::from_millis(request.timeout_ms);
    let poll = Duration::from_millis(request.poll_ms);
    let mut attempts = 0_u32;
    let mut last_snapshot = None;
    let mut last_matches = MacControlTargetMatches::default();
    let mut last_error = None;

    loop {
        attempts = attempts.saturating_add(1);
        match bridge
            .snapshot(MacControlSnapshotRequest {
                include_screenshot: false,
                max_elements: request.max_elements,
                max_depth: request.max_depth,
            })
            .await
        {
            Ok(snapshot) => {
                let matches = find_target_matches(&snapshot, &target);
                let matched = target_matches(&target, &matches);
                record_snapshot(snapshot.clone());
                if matched {
                    return MacControlWaitResponse {
                        status,
                        matched: true,
                        elapsed_ms: elapsed_ms(started),
                        attempts,
                        target,
                        matches,
                        snapshot: Some(snapshot),
                        error: None,
                    };
                }
                last_matches = matches;
                last_snapshot = Some(snapshot);
            }
            Err(error) => {
                last_error = Some(error);
            }
        }

        if started.elapsed() >= timeout {
            return MacControlWaitResponse {
                status,
                matched: false,
                elapsed_ms: elapsed_ms(started),
                attempts,
                target,
                matches: last_matches,
                snapshot: last_snapshot,
                error: Some(last_error.unwrap_or_else(|| {
                    format!(
                        "Timed out waiting for macOS target after {} ms.",
                        request.timeout_ms
                    )
                })),
            };
        }

        let remaining = timeout.saturating_sub(started.elapsed());
        tokio::time::sleep(poll.min(remaining)).await;
    }
}

pub async fn apps(request: MacControlAppsRequest) -> MacControlAppsResponse {
    let request = request.clamped();
    let Some(bridge) = available_bridge() else {
        return unsupported_apps_response(unsupported_reason());
    };
    let system_permissions = bridge.system_permissions().await;
    let status = status_from_system_permissions(true, true, system_permissions.clone());
    if !system_permissions.supported {
        return MacControlAppsResponse {
            status,
            result: None,
            error: Some("macOS control is unsupported in this runtime.".to_string()),
        };
    }
    if request.op == MacControlAppsOp::Activate && !activate_request_has_target(&request) {
        return MacControlAppsResponse {
            status,
            result: None,
            error: Some(
                "mac_control apps.activate requires one of pid, bundleId, or appName.".to_string(),
            ),
        };
    }

    match bridge.apps(request).await {
        Ok(result) => MacControlAppsResponse {
            status,
            result: Some(result),
            error: None,
        },
        Err(error) => MacControlAppsResponse {
            status,
            result: None,
            error: Some(error),
        },
    }
}

pub async fn capture_frame() -> MacControlFrameResponse {
    let Some(bridge) = available_bridge() else {
        return unsupported_frame_response(unsupported_reason());
    };
    let system_permissions = bridge.system_permissions().await;
    let status = status_from_system_permissions(true, true, system_permissions.clone());
    if !system_permissions.supported {
        return MacControlFrameResponse {
            status,
            frame: None,
            error: Some("macOS control is unsupported in this runtime.".to_string()),
        };
    }
    if !permission_granted(&system_permissions, "screen_recording") {
        return MacControlFrameResponse {
            status,
            frame: None,
            error: Some(
                "Mac Control frame capture requires Screen Recording permission.".to_string(),
            ),
        };
    }

    match bridge.capture_frame().await {
        Ok(frame) => {
            emit_frame(&frame);
            MacControlFrameResponse {
                status,
                frame: Some(frame),
                error: None,
            }
        }
        Err(error) => MacControlFrameResponse {
            status,
            frame: None,
            error: Some(error),
        },
    }
}

pub fn store_screenshot_jpeg(
    media_id: &str,
    bytes: &[u8],
    width_px: u32,
    height_px: u32,
) -> Result<MacControlScreenshotSummary, String> {
    let media_id = sanitize_media_id(media_id)?;
    let dir = crate::paths::mac_control_snapshots_dir()
        .map_err(|e| format!("Unable to resolve macOS control snapshots directory: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| {
        format!(
            "Unable to create macOS control snapshots directory {}: {e}",
            dir.display()
        )
    })?;
    let path = dir.join(format!("{media_id}.jpg"));
    fs::write(&path, bytes).map_err(|e| {
        format!(
            "Unable to write macOS control screenshot {}: {e}",
            path.display()
        )
    })?;
    prune_screenshot_files(&dir);

    Ok(MacControlScreenshotSummary {
        media_id,
        path: path.display().to_string(),
        width_px,
        height_px,
    })
}

pub fn emit_frame(payload: &MacControlFramePayload) {
    if let Some(bus) = crate::globals::get_event_bus() {
        match serde_json::to_value(payload) {
            Ok(value) => bus.emit(EVENT_MAC_CONTROL_FRAME, value),
            Err(e) => app_warn!(
                "mac_control",
                "frame",
                "Failed to serialize MacControlFramePayload: {}",
                e
            ),
        }
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

pub fn unsupported_wait_response(
    message: &str,
    target: MacControlTargetQuery,
) -> MacControlWaitResponse {
    MacControlWaitResponse {
        status: unsupported_status(message),
        matched: false,
        elapsed_ms: 0,
        attempts: 0,
        target,
        matches: MacControlTargetMatches::default(),
        snapshot: None,
        error: Some(message.to_string()),
    }
}

pub fn unsupported_apps_response(message: &str) -> MacControlAppsResponse {
    MacControlAppsResponse {
        status: unsupported_status(message),
        result: None,
        error: Some(message.to_string()),
    }
}

pub fn unsupported_frame_response(message: &str) -> MacControlFrameResponse {
    MacControlFrameResponse {
        status: unsupported_status(message),
        frame: None,
        error: Some(message.to_string()),
    }
}

fn activate_request_has_target(request: &MacControlAppsRequest) -> bool {
    request.pid.is_some()
        || request
            .bundle_id
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        || request
            .app_name
            .as_deref()
            .is_some_and(|value| !value.is_empty())
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

pub fn new_snapshot_id() -> String {
    format!("macsnap_{}", Uuid::new_v4().simple())
}

fn snapshot_cache() -> &'static Mutex<VecDeque<MacControlSnapshot>> {
    SNAPSHOT_CACHE.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn record_snapshot(snapshot: MacControlSnapshot) {
    let Ok(mut cache) = snapshot_cache().lock() else {
        return;
    };
    cache.push_back(snapshot);
    while cache.len() > MAX_SNAPSHOT_CACHE {
        cache.pop_front();
    }
}

pub fn cached_snapshot(snapshot_id: &str) -> Option<MacControlSnapshot> {
    let cache = snapshot_cache().lock().ok()?;
    cache
        .iter()
        .rev()
        .find(|snapshot| snapshot.snapshot_id == snapshot_id)
        .cloned()
}

fn find_target_matches(
    snapshot: &MacControlSnapshot,
    target: &MacControlTargetQuery,
) -> MacControlTargetMatches {
    let app_matches = snapshot
        .frontmost_app
        .as_ref()
        .filter(|app| app_matches_target(app, target))
        .cloned();
    let windows = snapshot
        .windows
        .iter()
        .filter(|window| window_matches_target(window, target))
        .cloned()
        .collect::<Vec<_>>();
    let elements = snapshot
        .elements
        .iter()
        .filter(|element| element_matches_target(element, target, snapshot))
        .cloned()
        .collect::<Vec<_>>();

    MacControlTargetMatches {
        app: app_matches,
        windows,
        elements,
    }
}

fn target_matches(target: &MacControlTargetQuery, matches: &MacControlTargetMatches) -> bool {
    if target.wants_app() && matches.app.is_none() {
        return false;
    }
    if target.wants_element() {
        return !matches.elements.is_empty();
    }
    if target
        .window_title
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        return !matches.windows.is_empty();
    }
    matches.app.is_some()
}

fn app_matches_target(app: &MacControlAppSummary, target: &MacControlTargetQuery) -> bool {
    optional_contains(app.name.as_deref(), target.app_name.as_deref())
        && optional_contains(app.bundle_id.as_deref(), target.bundle_id.as_deref())
}

fn window_matches_target(window: &MacControlWindowSummary, target: &MacControlTargetQuery) -> bool {
    optional_contains(window.title.as_deref(), target.window_title.as_deref())
}

fn element_matches_target(
    element: &MacControlElementSummary,
    target: &MacControlTargetQuery,
    snapshot: &MacControlSnapshot,
) -> bool {
    if !optional_eq(element.id.as_str(), target.element_id.as_deref()) {
        return false;
    }
    if !optional_contains(element.role.as_deref(), target.role.as_deref()) {
        return false;
    }
    if !optional_contains_any(
        &[element.label.as_deref(), element.value.as_deref()],
        target.text.as_deref(),
    ) {
        return false;
    }
    if target
        .enabled
        .is_some_and(|enabled| element.enabled != Some(enabled))
    {
        return false;
    }
    if target
        .focused
        .is_some_and(|focused| element.focused != focused)
    {
        return false;
    }
    if !target
        .window_title
        .as_deref()
        .filter(|query| !query.is_empty())
        .map(|query| {
            element
                .window_id
                .as_deref()
                .and_then(|window_id| {
                    snapshot
                        .windows
                        .iter()
                        .find(|window| window.id == window_id)
                })
                .is_some_and(|window| contains_ci(window.title.as_deref(), query))
        })
        .unwrap_or(true)
    {
        return false;
    }
    true
}

fn optional_eq(actual: &str, query: Option<&str>) -> bool {
    query
        .filter(|query| !query.is_empty())
        .map_or(true, |query| actual == query)
}

fn optional_contains(actual: Option<&str>, query: Option<&str>) -> bool {
    query
        .filter(|query| !query.is_empty())
        .map_or(true, |query| contains_ci(actual, query))
}

fn optional_contains_any(actuals: &[Option<&str>], query: Option<&str>) -> bool {
    query
        .filter(|query| !query.is_empty())
        .map_or(true, |query| {
            actuals.iter().any(|actual| contains_ci(*actual, query))
        })
}

fn contains_ci(actual: Option<&str>, query: &str) -> bool {
    actual
        .map(|actual| {
            actual
                .to_ascii_lowercase()
                .contains(&query.to_ascii_lowercase())
        })
        .unwrap_or(false)
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn sanitize_media_id(media_id: &str) -> Result<String, String> {
    let sanitized = media_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
        .collect::<String>();
    if sanitized.is_empty() {
        Err("macOS control screenshot media id is empty after sanitization.".to_string())
    } else {
        Ok(sanitized)
    }
}

fn prune_screenshot_files(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "jpg" | "jpeg"))
                .unwrap_or(false)
        })
        .map(|path| {
            let modified = path
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (modified, path)
        })
        .collect::<Vec<(SystemTime, PathBuf)>>();
    if files.len() <= MAX_SCREENSHOT_FILES {
        return;
    }
    files.sort_by_key(|(modified, _)| *modified);
    let remove_count = files.len().saturating_sub(MAX_SCREENSHOT_FILES);
    for (_, path) in files.into_iter().take(remove_count) {
        let _ = fs::remove_file(path);
    }
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

    fn sample_snapshot() -> MacControlSnapshot {
        MacControlSnapshot {
            snapshot_id: "macsnap_sample".to_string(),
            created_at: "2026-05-17T00:00:00Z".to_string(),
            frontmost_app: Some(MacControlAppSummary {
                pid: 42,
                bundle_id: Some("com.apple.finder".to_string()),
                name: Some("Finder".to_string()),
            }),
            displays: Vec::new(),
            windows: vec![MacControlWindowSummary {
                id: "win_1".to_string(),
                app_pid: Some(42),
                title: Some("Downloads".to_string()),
                focused: true,
                bounds_points: None,
            }],
            elements: vec![
                MacControlElementSummary {
                    id: "el_1".to_string(),
                    window_id: Some("win_1".to_string()),
                    role: Some("AXButton".to_string()),
                    label: Some("Open".to_string()),
                    value: None,
                    enabled: Some(true),
                    focused: false,
                    bounds_points: None,
                    actions: vec!["AXPress".to_string()],
                },
                MacControlElementSummary {
                    id: "el_2".to_string(),
                    window_id: Some("win_1".to_string()),
                    role: Some("AXTextField".to_string()),
                    label: None,
                    value: Some("Search Downloads".to_string()),
                    enabled: Some(true),
                    focused: true,
                    bounds_points: None,
                    actions: Vec::new(),
                },
            ],
            screenshot: None,
            truncated: false,
            warnings: Vec::new(),
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
    fn wait_request_clamps_limits() {
        let request = MacControlWaitRequest {
            timeout_ms: 500_000,
            poll_ms: 1,
            max_elements: 10_000,
            max_depth: 100,
            ..Default::default()
        }
        .clamped();

        assert_eq!(request.timeout_ms, HARD_WAIT_TIMEOUT_MS);
        assert_eq!(request.poll_ms, MIN_WAIT_POLL_MS);
        assert_eq!(request.max_elements, HARD_SNAPSHOT_MAX_ELEMENTS);
        assert_eq!(request.max_depth, HARD_SNAPSHOT_MAX_DEPTH);
    }

    #[test]
    fn apps_request_clamps_limit() {
        let request = MacControlAppsRequest {
            limit: 10_000,
            ..Default::default()
        }
        .clamped();

        assert_eq!(request.limit, 100);
        assert_eq!(request.op, MacControlAppsOp::Frontmost);
    }

    #[test]
    fn activate_request_requires_target() {
        let missing = MacControlAppsRequest {
            op: MacControlAppsOp::Activate,
            ..Default::default()
        };
        let by_name = MacControlAppsRequest {
            op: MacControlAppsOp::Activate,
            app_name: Some("Finder".to_string()),
            ..Default::default()
        };

        assert!(!activate_request_has_target(&missing));
        assert!(activate_request_has_target(&by_name));
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

    #[test]
    fn unsupported_wait_response_has_consistent_shape() {
        let target = MacControlTargetQuery {
            app_name: Some("Finder".to_string()),
            ..Default::default()
        };
        let response = unsupported_wait_response("no wait bridge", target);

        assert_eq!(response.status.readiness, MacControlReadiness::Unsupported);
        assert!(!response.status.supported);
        assert!(!response.matched);
        assert!(response.snapshot.is_none());
        assert_eq!(response.error.as_deref(), Some("no wait bridge"));
    }

    #[test]
    fn unsupported_apps_response_has_consistent_shape() {
        let response = unsupported_apps_response("no apps bridge");

        assert_eq!(response.status.readiness, MacControlReadiness::Unsupported);
        assert!(!response.status.supported);
        assert!(response.result.is_none());
        assert_eq!(response.error.as_deref(), Some("no apps bridge"));
    }

    #[test]
    fn unsupported_frame_response_has_consistent_shape() {
        let response = unsupported_frame_response("no frame bridge");

        assert_eq!(response.status.readiness, MacControlReadiness::Unsupported);
        assert!(!response.status.supported);
        assert!(response.frame.is_none());
        assert_eq!(response.error.as_deref(), Some("no frame bridge"));
    }

    #[test]
    fn snapshot_cache_keeps_newest_entries() {
        {
            let mut cache = snapshot_cache().lock().expect("snapshot cache lock");
            cache.clear();
        }

        for idx in 0..(MAX_SNAPSHOT_CACHE + 2) {
            let mut snapshot = MacControlSnapshot::new_empty();
            snapshot.snapshot_id = format!("macsnap_test_{idx}");
            record_snapshot(snapshot);
        }

        let cache = snapshot_cache().lock().expect("snapshot cache lock");
        assert_eq!(cache.len(), MAX_SNAPSHOT_CACHE);
        assert_eq!(
            cache.front().map(|snapshot| snapshot.snapshot_id.as_str()),
            Some("macsnap_test_2")
        );
        assert_eq!(
            cache.back().map(|snapshot| snapshot.snapshot_id.as_str()),
            Some("macsnap_test_21")
        );
    }

    #[test]
    fn target_query_matches_combined_app_window_and_element_filters() {
        let snapshot = sample_snapshot();
        let target = MacControlTargetQuery {
            app_name: Some("find".to_string()),
            window_title: Some("down".to_string()),
            text: Some("open".to_string()),
            role: Some("button".to_string()),
            enabled: Some(true),
            ..Default::default()
        };

        let matches = find_target_matches(&snapshot, &target);

        assert!(target_matches(&target, &matches));
        assert_eq!(
            matches.app.as_ref().and_then(|app| app.name.as_deref()),
            Some("Finder")
        );
        assert_eq!(matches.windows.len(), 1);
        assert_eq!(matches.elements.len(), 1);
        assert_eq!(matches.elements[0].id, "el_1");
    }

    #[test]
    fn target_query_requires_app_filter_even_when_element_matches() {
        let snapshot = sample_snapshot();
        let target = MacControlTargetQuery {
            app_name: Some("Safari".to_string()),
            text: Some("open".to_string()),
            ..Default::default()
        };

        let matches = find_target_matches(&snapshot, &target);

        assert!(!target_matches(&target, &matches));
        assert!(matches.app.is_none());
        assert_eq!(matches.elements.len(), 1);
    }
}
