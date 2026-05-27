//! macOS desktop control bridge and readiness model.
//!
//! Exposes status / permissions, Accessibility snapshots, scored element
//! search, display/window screenshot frames, wait/target matching, app
//! focus/launch, Dock and Spaces helpers, window operations, AX-first element
//! actions, clipboard text, dialogs, and menu inspection/clicks.

use std::{
    collections::{HashMap, VecDeque},
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
const MAX_ERROR_STATS: usize = 20;
const DEFAULT_ELEMENTS_LIMIT: usize = 20;
const HARD_ELEMENTS_LIMIT: usize = 100;
const DEFAULT_WAIT_TIMEOUT_MS: u64 = 10_000;
const DEFAULT_WAIT_POLL_MS: u64 = 500;
const HARD_WAIT_TIMEOUT_MS: u64 = 60_000;
const MIN_WAIT_POLL_MS: u64 = 100;
const HARD_WAIT_POLL_MS: u64 = 5_000;
const DEFAULT_CLIPBOARD_MAX_CHARS: usize = 4_000;
const HARD_CLIPBOARD_MAX_CHARS: usize = 20_000;
const HARD_CLIPBOARD_SET_CHARS: usize = 200_000;
const DEFAULT_VISUAL_LIMIT: usize = 5;
const DEFAULT_UI_MAP_LIMIT: usize = 80;
const HARD_UI_MAP_LIMIT: usize = 200;
const DEFAULT_DIAGNOSTICS_LIMIT: usize = 10;
const HARD_DIAGNOSTICS_LIMIT: usize = 20;
const HARD_MOTION_STEPS: usize = 240;
const HARD_MOTION_DURATION_MS: u64 = 10_000;
const HARD_TYPING_DELAY_MS: u64 = 1_000;
const HARD_PRESS_REPEAT: usize = 100;
const HARD_PRESS_INTERVAL_MS: u64 = 5_000;
const HARD_PRESS_HOLD_MS: u64 = 10_000;
const DEFAULT_MENU_POPOVER_LIMIT: usize = 5;
const HARD_MENU_POPOVER_LIMIT: usize = 20;
pub const ALLOWED_PERFORM_AX_ACTIONS: &[&str] = &[
    "AXPress",
    "AXShowMenu",
    "AXConfirm",
    "AXCancel",
    "AXIncrement",
    "AXDecrement",
    "AXPick",
    "AXRaise",
    "AXShowDefaultUI",
    "AXShowAlternateUI",
];
pub const EVENT_MAC_CONTROL_FRAME: &str = "mac_control:frame";

#[async_trait]
pub trait MacControlBridge: Send + Sync {
    async fn system_permissions(&self) -> SystemPermissionsResponse;
    async fn snapshot(
        &self,
        request: MacControlSnapshotRequest,
    ) -> Result<MacControlSnapshot, String>;
    async fn elements(
        &self,
        request: MacControlElementsRequest,
    ) -> Result<MacControlElementsResult, String>;
    async fn capture_frame(&self) -> Result<MacControlFramePayload, String>;
    async fn apps(&self, request: MacControlAppsRequest) -> Result<MacControlAppsResult, String>;
    async fn dock(&self, request: MacControlDockRequest) -> Result<MacControlDockResult, String>;
    async fn spaces(
        &self,
        request: MacControlSpacesRequest,
    ) -> Result<MacControlSpacesResult, String>;
    async fn windows(
        &self,
        request: MacControlWindowsRequest,
    ) -> Result<MacControlWindowsResult, String>;
    async fn act(&self, request: MacControlActRequest) -> Result<MacControlActResult, String>;
    async fn menu(&self, request: MacControlMenuRequest) -> Result<MacControlMenuResult, String>;
    async fn clipboard(
        &self,
        request: MacControlClipboardRequest,
    ) -> Result<MacControlClipboardResult, String>;
    async fn dialog(
        &self,
        request: MacControlDialogRequest,
    ) -> Result<MacControlDialogResult, String>;
    async fn ocr(
        &self,
        request: MacControlOcrRequest,
    ) -> Result<Vec<MacControlOcrRawTextBlock>, String>;
}

static MAC_CONTROL_BRIDGE: OnceLock<Arc<dyn MacControlBridge>> = OnceLock::new();
static SNAPSHOT_CACHE: OnceLock<Mutex<VecDeque<MacControlSnapshot>>> = OnceLock::new();
static ERROR_STATS: OnceLock<Mutex<HashMap<String, MacControlErrorStat>>> = OnceLock::new();

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
    pub stats: MacControlRuntimeStats,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlRuntimeStats {
    pub snapshot_cache_len: usize,
    pub snapshot_cache_limit: usize,
    pub screenshot_file_limit: usize,
    pub recent_errors: Vec<MacControlErrorStat>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlFocusAnchor {
    pub pid: i32,
    pub bundle_id: Option<String>,
    pub name: Option<String>,
    pub focused_window_id: Option<String>,
    pub focused_window_title: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlErrorStat {
    pub operation: String,
    pub message: String,
    pub count: u64,
    pub last_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlPermissionsResponse {
    pub status: MacControlStatus,
    pub system_permissions: SystemPermissionsResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlDiagnosticsResponse {
    pub status: MacControlStatus,
    pub result: Option<MacControlDiagnosticsResult>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlDiagnosticsResult {
    pub op: MacControlDiagnosticsOp,
    pub generated_at: String,
    pub snapshot_cache: Vec<MacControlCachedSnapshotSummary>,
    pub recent_errors: Vec<MacControlErrorStat>,
    pub focus_anchor: Option<MacControlFocusAnchor>,
    pub export_path: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlCachedSnapshotSummary {
    pub snapshot_id: String,
    pub created_at: String,
    pub frontmost_app: Option<MacControlAppSummary>,
    pub display_count: usize,
    pub window_count: usize,
    pub element_count: usize,
    pub has_screenshot: bool,
    pub screenshot: Option<MacControlScreenshotSummary>,
    pub truncated: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlSnapshotRequest {
    #[serde(default)]
    pub include_screenshot: bool,
    #[serde(default)]
    pub screenshot_target: MacControlScreenshotTarget,
    #[serde(default)]
    pub display_id: Option<u32>,
    #[serde(default)]
    pub window_id: Option<String>,
    #[serde(default = "default_snapshot_max_elements")]
    pub max_elements: usize,
    #[serde(default = "default_snapshot_max_depth")]
    pub max_depth: usize,
}

impl Default for MacControlSnapshotRequest {
    fn default() -> Self {
        Self {
            include_screenshot: false,
            screenshot_target: MacControlScreenshotTarget::Display,
            display_id: None,
            window_id: None,
            max_elements: DEFAULT_SNAPSHOT_MAX_ELEMENTS,
            max_depth: DEFAULT_SNAPSHOT_MAX_DEPTH,
        }
    }
}

impl MacControlSnapshotRequest {
    pub fn clamped(mut self) -> Self {
        self.window_id = normalize_optional_string(self.window_id);
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlScreenshotTarget {
    #[serde(alias = "screen")]
    #[default]
    Display,
    #[serde(alias = "frontmost_window")]
    Window,
}

fn default_snapshot_max_elements() -> usize {
    DEFAULT_SNAPSHOT_MAX_ELEMENTS
}

fn default_snapshot_max_depth() -> usize {
    DEFAULT_SNAPSHOT_MAX_DEPTH
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlDiagnosticsRequest {
    #[serde(default)]
    pub op: MacControlDiagnosticsOp,
    #[serde(default = "default_diagnostics_limit")]
    pub limit: usize,
}

impl Default for MacControlDiagnosticsRequest {
    fn default() -> Self {
        Self {
            op: MacControlDiagnosticsOp::Summary,
            limit: DEFAULT_DIAGNOSTICS_LIMIT,
        }
    }
}

impl MacControlDiagnosticsRequest {
    pub fn clamped(mut self) -> Self {
        if self.limit == 0 {
            self.limit = DEFAULT_DIAGNOSTICS_LIMIT;
        }
        self.limit = self.limit.min(HARD_DIAGNOSTICS_LIMIT);
        self
    }
}

fn default_diagnostics_limit() -> usize {
    DEFAULT_DIAGNOSTICS_LIMIT
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlDiagnosticsOp {
    #[default]
    Summary,
    Export,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlElementsRequest {
    #[serde(default)]
    pub op: MacControlElementsOp,
    #[serde(default)]
    pub target: MacControlTargetQuery,
    #[serde(default = "default_elements_limit")]
    pub limit: usize,
    #[serde(default = "default_snapshot_max_elements")]
    pub max_elements: usize,
    #[serde(default = "default_snapshot_max_depth")]
    pub max_depth: usize,
}

impl MacControlElementsRequest {
    pub fn clamped(mut self) -> Self {
        self.target = self.target.normalized();
        if self.limit == 0 {
            self.limit = DEFAULT_ELEMENTS_LIMIT;
        }
        if self.max_elements == 0 {
            self.max_elements = DEFAULT_SNAPSHOT_MAX_ELEMENTS;
        }
        if self.max_depth == 0 {
            self.max_depth = DEFAULT_SNAPSHOT_MAX_DEPTH;
        }
        self.limit = self.limit.min(HARD_ELEMENTS_LIMIT);
        self.max_elements = self.max_elements.min(HARD_SNAPSHOT_MAX_ELEMENTS);
        self.max_depth = self.max_depth.min(HARD_SNAPSHOT_MAX_DEPTH);
        self
    }
}

fn default_elements_limit() -> usize {
    DEFAULT_ELEMENTS_LIMIT
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlElementsOp {
    #[default]
    Find,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlVisualRequest {
    #[serde(default)]
    pub op: MacControlVisualOp,
    #[serde(default)]
    pub snapshot_id: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub text_match: MacControlStringMatch,
    #[serde(default)]
    pub coordinate_space: MacControlCoordinateSpace,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub screenshot_target: MacControlScreenshotTarget,
    #[serde(default)]
    pub display_id: Option<u32>,
    #[serde(default)]
    pub window_id: Option<String>,
    #[serde(default = "default_snapshot_max_elements")]
    pub max_elements: usize,
    #[serde(default = "default_snapshot_max_depth")]
    pub max_depth: usize,
    #[serde(default = "default_visual_limit")]
    pub limit: usize,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub min_confidence: Option<f32>,
    #[serde(default)]
    pub recognition_level: MacControlOcrRecognitionLevel,
    #[serde(default)]
    pub annotate: bool,
    #[serde(default = "default_ui_map_limit")]
    pub ui_map_limit: usize,
}

impl MacControlVisualRequest {
    pub fn clamped(mut self) -> Self {
        self.snapshot_id = normalize_optional_string(self.snapshot_id);
        self.text = normalize_optional_string(self.text);
        self.window_id = normalize_optional_string(self.window_id);
        self.languages = self
            .languages
            .into_iter()
            .filter_map(|language| normalize_optional_string(Some(language)))
            .take(16)
            .collect();
        self.min_confidence = self
            .min_confidence
            .filter(|value| value.is_finite())
            .map(|value| value.clamp(0.0, 1.0));
        if self.max_elements == 0 {
            self.max_elements = DEFAULT_SNAPSHOT_MAX_ELEMENTS;
        }
        if self.max_depth == 0 {
            self.max_depth = DEFAULT_SNAPSHOT_MAX_DEPTH;
        }
        if self.limit == 0 {
            self.limit = DEFAULT_VISUAL_LIMIT;
        }
        if self.ui_map_limit == 0 {
            self.ui_map_limit = DEFAULT_UI_MAP_LIMIT;
        }
        self.max_elements = self.max_elements.min(HARD_SNAPSHOT_MAX_ELEMENTS);
        self.max_depth = self.max_depth.min(HARD_SNAPSHOT_MAX_DEPTH);
        self.limit = self.limit.min(HARD_ELEMENTS_LIMIT);
        self.ui_map_limit = self.ui_map_limit.min(HARD_UI_MAP_LIMIT);
        self
    }
}

fn default_visual_limit() -> usize {
    DEFAULT_VISUAL_LIMIT
}

fn default_ui_map_limit() -> usize {
    DEFAULT_UI_MAP_LIMIT
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlVisualOp {
    #[default]
    Observe,
    Point,
    Ocr,
    FindText,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlCoordinateSpace {
    #[default]
    ImagePixels,
    ScreenPoints,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlOcrRecognitionLevel {
    Fast,
    #[default]
    Accurate,
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
pub struct MacControlElementsResponse {
    pub status: MacControlStatus,
    pub result: Option<MacControlElementsResult>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlElementsResult {
    pub op: MacControlElementsOp,
    pub target: MacControlTargetQuery,
    pub snapshot_id: String,
    pub created_at: String,
    pub frontmost_app: Option<MacControlAppSummary>,
    pub total_matches: usize,
    pub elements: Vec<MacControlElementCandidate>,
    pub truncated: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlElementCandidate {
    pub element: MacControlElementSummary,
    pub window: Option<MacControlWindowSummary>,
    pub score: u8,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlWaitResponse {
    pub status: MacControlStatus,
    pub op: MacControlWaitOp,
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
pub struct MacControlDockResponse {
    pub status: MacControlStatus,
    pub result: Option<MacControlDockResult>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlSpacesResponse {
    pub status: MacControlStatus,
    pub result: Option<MacControlSpacesResult>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlAppsResult {
    pub op: MacControlAppsOp,
    pub frontmost: Option<MacControlRunningApp>,
    pub apps: Vec<MacControlRunningApp>,
    pub installed_apps: Vec<MacControlInstalledApp>,
    pub activated: Option<MacControlRunningApp>,
    pub launched: Option<MacControlRunningApp>,
    pub quit: Option<MacControlRunningApp>,
    pub execution: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlDockResult {
    pub op: MacControlDockOp,
    pub autohide: Option<bool>,
    pub orientation: Option<String>,
    pub items: Vec<MacControlDockItem>,
    pub launched: Option<MacControlDockItem>,
    pub menu_items: Vec<MacControlMenuItemSummary>,
    pub selected_menu_item: Option<MacControlMenuItemSummary>,
    pub execution: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlDockItem {
    pub id: String,
    pub index: usize,
    pub section: MacControlDockSection,
    pub tile_type: Option<String>,
    pub label: Option<String>,
    pub bundle_id: Option<String>,
    pub path: Option<String>,
    pub running: bool,
    pub pid: Option<i32>,
    pub active: bool,
    pub hidden: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlDockSection {
    PersistentApps,
    PersistentOthers,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlSpacesResult {
    pub op: MacControlSpacesOp,
    pub displays: Vec<MacControlSpacesDisplay>,
    pub switched: Option<MacControlSpaceSummary>,
    pub moved_window: Option<MacControlWindowSummary>,
    pub execution: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlSpacesDisplay {
    pub display_identifier: Option<String>,
    pub current_space: Option<MacControlSpaceSummary>,
    pub spaces: Vec<MacControlSpaceSummary>,
    pub collapsed_space: Option<MacControlSpaceSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlSpaceSummary {
    pub id: Option<u64>,
    pub uuid: Option<String>,
    pub index: usize,
    pub kind: Option<String>,
    pub current: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlInstalledApp {
    pub name: Option<String>,
    pub bundle_id: Option<String>,
    pub path: Option<String>,
    pub executable_path: Option<String>,
    pub running: bool,
    pub pid: Option<i32>,
    pub active: bool,
    pub hidden: bool,
    pub activation_policy: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlWindowsResponse {
    pub status: MacControlStatus,
    pub result: Option<MacControlWindowsResult>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlWindowsResult {
    pub op: MacControlWindowsOp,
    pub window_scope: MacControlWindowsScope,
    pub frontmost_app: Option<MacControlAppSummary>,
    pub windows: Vec<MacControlWindowSummary>,
    pub acted_window: Option<MacControlWindowSummary>,
    pub execution: Option<String>,
    pub verification: Option<MacControlVerification>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlActResponse {
    pub status: MacControlStatus,
    pub result: Option<MacControlActResult>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlActResult {
    pub op: MacControlActOp,
    pub execution: String,
    pub performed_action: Option<String>,
    pub target: Option<MacControlElementSummary>,
    pub snapshot: Option<MacControlSnapshot>,
    pub verification: Option<MacControlVerification>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlVerification {
    pub status: MacControlVerificationStatus,
    pub summary: String,
    pub checks: Vec<MacControlVerificationCheck>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlVerificationStatus {
    Verified,
    Failed,
    Unverified,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlVerificationCheck {
    pub name: String,
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlMenuResponse {
    pub status: MacControlStatus,
    pub result: Option<MacControlMenuResult>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlClipboardResponse {
    pub status: MacControlStatus,
    pub result: Option<MacControlClipboardResult>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlDialogResponse {
    pub status: MacControlStatus,
    pub result: Option<MacControlDialogResult>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlVisualResponse {
    pub status: MacControlStatus,
    pub result: Option<MacControlVisualResult>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlVisualResult {
    pub op: MacControlVisualOp,
    pub snapshot_id: Option<String>,
    pub snapshot: Option<MacControlSnapshot>,
    pub screenshot: Option<MacControlScreenshotSummary>,
    pub annotated_screenshot: Option<MacControlScreenshotSummary>,
    pub ui_map: Vec<MacControlUiMapItem>,
    pub coordinate_space: Option<MacControlCoordinateSpace>,
    pub image_point: Option<MacControlPoint>,
    pub screen_point: Option<MacControlPoint>,
    pub inside_frame: Option<bool>,
    pub hit_elements: Vec<MacControlVisualElementMatch>,
    pub nearest_elements: Vec<MacControlVisualElementMatch>,
    pub text_blocks: Vec<MacControlOcrTextBlock>,
    pub text_matches: Vec<MacControlOcrTextMatch>,
    pub suggested_action: Option<MacControlSuggestedAction>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlVisualElementMatch {
    pub element: MacControlElementSummary,
    pub window: Option<MacControlWindowSummary>,
    pub contains_point: bool,
    pub distance_points: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlUiMapItem {
    pub id: String,
    pub window_id: Option<String>,
    pub role: Option<String>,
    pub text: Option<String>,
    pub enabled: Option<bool>,
    pub focused: bool,
    pub bounds_points: MacControlBounds,
    pub image_bounds: MacControlBounds,
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlOcrTextBlock {
    pub id: String,
    pub text: String,
    pub confidence: f32,
    pub image_bounds: MacControlBounds,
    pub screen_bounds: MacControlBounds,
    pub image_point: MacControlPoint,
    pub screen_point: MacControlPoint,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlOcrTextMatch {
    pub block: MacControlOcrTextBlock,
    pub score: u8,
    pub reasons: Vec<String>,
    pub hit_elements: Vec<MacControlVisualElementMatch>,
    pub nearest_elements: Vec<MacControlVisualElementMatch>,
    pub suggested_action: Option<MacControlSuggestedAction>,
}

#[derive(Debug, Clone)]
pub struct MacControlOcrRequest {
    pub screenshot: MacControlScreenshotSummary,
    pub languages: Vec<String>,
    pub recognition_level: MacControlOcrRecognitionLevel,
}

#[derive(Debug, Clone)]
pub struct MacControlOcrRawTextBlock {
    pub text: String,
    pub confidence: f32,
    pub image_bounds: MacControlBounds,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlSuggestedAction {
    pub action: String,
    pub op: MacControlActOp,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlMenuResult {
    pub op: MacControlMenuOp,
    pub scope: MacControlMenuScope,
    pub path: Vec<String>,
    pub items: Vec<MacControlMenuItemSummary>,
    pub clicked: Option<MacControlMenuItemSummary>,
    pub popovers: Vec<MacControlMenuPopoverCandidate>,
    pub screenshot: Option<MacControlScreenshotSummary>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlMenuItemSummary {
    pub id: Option<String>,
    pub index: Option<usize>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub value: Option<String>,
    pub role: Option<String>,
    pub enabled: Option<bool>,
    pub bounds_points: Option<MacControlBounds>,
    pub actions: Vec<String>,
    pub children: Vec<MacControlMenuItemSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlMenuPopoverCandidate {
    pub window: MacControlWindowSummary,
    pub app: Option<MacControlAppSummary>,
    pub score: u8,
    pub reasons: Vec<String>,
    pub ocr_text: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlClipboardResult {
    pub op: MacControlClipboardOp,
    pub text: Option<String>,
    pub text_len: usize,
    pub truncated: bool,
    pub changed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlDialogResult {
    pub op: MacControlDialogOp,
    pub dialogs: Vec<MacControlDialogSummary>,
    pub acted_button: Option<MacControlElementSummary>,
    pub acted_field: Option<MacControlElementSummary>,
    pub file_dialog: Option<MacControlDialogFileResult>,
    pub snapshot: Option<MacControlSnapshot>,
    pub execution: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlDialogSummary {
    pub window: MacControlWindowSummary,
    pub text: Vec<String>,
    pub buttons: Vec<MacControlElementSummary>,
    pub fields: Vec<MacControlElementSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlDialogFileResult {
    pub path: Option<String>,
    pub name: Option<String>,
    pub requested_button: Option<String>,
    pub selected_button: Option<String>,
    pub name_field: Option<MacControlElementSummary>,
    pub path_navigation: Option<String>,
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
    pub role: Option<String>,
    pub subrole: Option<String>,
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
    pub target: MacControlScreenshotTarget,
    pub display_id: Option<u32>,
    pub window_id: Option<String>,
    pub window_title: Option<String>,
    pub bounds_points: Option<MacControlBounds>,
    pub scale: Option<f64>,
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
    pub target: MacControlScreenshotTarget,
    pub display_id: Option<u32>,
    pub window_id: Option<String>,
    pub window_title: Option<String>,
    pub bounds_points: Option<MacControlBounds>,
    pub scale: Option<f64>,
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
    pub app_name_match: MacControlAppNameMatch,
    #[serde(default)]
    pub bundle_id: Option<String>,
    #[serde(default)]
    pub pid: Option<i32>,
    #[serde(default = "default_apps_limit")]
    pub limit: usize,
}

impl MacControlAppsRequest {
    pub fn clamped(mut self) -> Self {
        self.app_name = normalize_optional_string(self.app_name);
        self.bundle_id = normalize_optional_string(self.bundle_id);
        if self.pid.is_some_and(|pid| pid <= 0) {
            self.pid = None;
        }
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
    Installed,
    Search,
    Activate,
    Launch,
    Quit,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlAppNameMatch {
    #[default]
    Exact,
    Contains,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlDockRequest {
    #[serde(default)]
    pub op: MacControlDockOp,
    #[serde(default)]
    pub dock_item_id: Option<String>,
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default)]
    pub app_name_match: MacControlAppNameMatch,
    #[serde(default)]
    pub bundle_id: Option<String>,
    #[serde(default)]
    pub item_path: Option<String>,
    #[serde(default)]
    pub menu_item: Option<String>,
    #[serde(default)]
    pub menu_index: Option<usize>,
    #[serde(default = "default_dock_limit")]
    pub limit: usize,
}

impl MacControlDockRequest {
    pub fn clamped(mut self) -> Self {
        self.dock_item_id = normalize_optional_string(self.dock_item_id);
        self.app_name = normalize_optional_string(self.app_name);
        self.bundle_id = normalize_optional_string(self.bundle_id);
        self.item_path = normalize_optional_string(self.item_path);
        self.menu_item = normalize_optional_string(self.menu_item);
        self.menu_index = self.menu_index.filter(|index| *index < 10_000);
        if self.limit == 0 {
            self.limit = default_dock_limit();
        }
        self.limit = self.limit.min(100);
        self
    }
}

fn default_dock_limit() -> usize {
    100
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlDockOp {
    #[default]
    List,
    Launch,
    Hide,
    Show,
    Menu,
    SelectMenu,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlSpacesRequest {
    #[serde(default)]
    pub op: MacControlSpacesOp,
    #[serde(default)]
    pub space_id: Option<u64>,
    #[serde(default)]
    pub space_index: Option<usize>,
    #[serde(default)]
    pub direction: Option<MacControlSpaceDirection>,
    #[serde(default)]
    pub window_id: Option<String>,
    #[serde(default)]
    pub target: MacControlTargetQuery,
    #[serde(default = "default_snapshot_max_elements")]
    pub max_elements: usize,
    #[serde(default = "default_snapshot_max_depth")]
    pub max_depth: usize,
}

impl MacControlSpacesRequest {
    pub fn clamped(mut self) -> Self {
        if self.space_id == Some(0) {
            self.space_id = None;
        }
        if self.space_index == Some(0) {
            self.space_index = None;
        }
        self.window_id = normalize_optional_string(self.window_id);
        self.target = self.target.normalized();
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

pub fn sanitize_tool_args(args: &serde_json::Value) -> serde_json::Value {
    let Some(action) = args.get("action").and_then(|value| value.as_str()) else {
        return args.clone();
    };
    let mut sanitized = args.clone();
    match action {
        "dock" => sanitize_dock_tool_args(&mut sanitized),
        "menu" => sanitize_menu_tool_args(&mut sanitized),
        "spaces" => sanitize_spaces_tool_args(&mut sanitized),
        _ => {}
    }
    sanitized
}

fn sanitize_dock_tool_args(args: &mut serde_json::Value) {
    let Some(object) = args.as_object_mut() else {
        return;
    };
    let op = object.get("op").and_then(|value| value.as_str());
    if op != Some("select_menu") {
        return;
    }
    if object.get("menuItem").is_some_and(has_non_empty_string) {
        object.remove("menuIndex");
    }
}

fn sanitize_menu_tool_args(args: &mut serde_json::Value) {
    let Some(object) = args.as_object_mut() else {
        return;
    };
    let op = object.get("op").and_then(|value| value.as_str());
    if op != Some("click") {
        return;
    }
    let has_path = object
        .get("path")
        .and_then(|value| value.as_array())
        .is_some_and(|items| items.iter().any(has_non_empty_string));
    if has_path {
        object.remove("menuIndex");
    }
}

fn has_non_empty_string(value: &serde_json::Value) -> bool {
    value.as_str().is_some_and(|text| !text.trim().is_empty())
}

fn sanitize_spaces_tool_args(args: &mut serde_json::Value) {
    let Some(object) = args.as_object_mut() else {
        return;
    };
    let op = object.get("op").and_then(|value| value.as_str());
    if op != Some("switch") {
        return;
    }

    let direction = object.get("direction").and_then(|value| value.as_str());
    let space_id = object.get("spaceId").and_then(|value| value.as_u64());
    let space_index = object.get("spaceIndex").and_then(|value| value.as_u64());

    match (direction, space_id, space_index) {
        // Legacy provider-filled default shape from the old schema:
        // direction=right plus spaceIndex=1. Prefer the explicit relative
        // direction so "switch right" from Space 1 still moves to Space 2.
        (Some("right"), None | Some(0), Some(1)) => {
            object.remove("spaceIndex");
        }
        // Exact targets win over a default direction. The schema now uses 0 as
        // the unset numeric default, so non-zero id/index is treated as intent.
        (Some(_), Some(id), _) if id > 0 => {
            object.remove("direction");
        }
        (Some(_), None | Some(0), Some(index)) if index > 0 => {
            object.remove("direction");
        }
        _ => {}
    }
}

pub fn preflight_tool_args(args: &serde_json::Value) -> Option<String> {
    let action = args.get("action").and_then(|value| value.as_str())?;
    match action {
        "apps" => {
            let request = match parse_preflight_request::<MacControlAppsRequest>(args, "apps") {
                Ok(request) => request.clamped(),
                Err(error) => return Some(error),
            };
            if matches!(
                request.op,
                MacControlAppsOp::Activate | MacControlAppsOp::Quit
            ) && !apps_request_has_target(&request)
            {
                return Some(format!(
                    "mac_control apps.{} requires one of pid, bundleId, or appName.",
                    apps_op_name(request.op)
                ));
            }
            if request.op == MacControlAppsOp::Launch && !apps_launch_request_has_target(&request) {
                return Some("mac_control apps.launch requires bundleId or appName.".to_string());
            }
            None
        }
        "dock" => {
            let request = match parse_preflight_request::<MacControlDockRequest>(args, "dock") {
                Ok(request) => request.clamped(),
                Err(error) => return Some(error),
            };
            validate_dock_request(&request)
        }
        "spaces" => match parse_preflight_request::<MacControlSpacesRequest>(args, "spaces") {
            Ok(request) => validate_spaces_request(&request.clamped()),
            Err(error) => Some(error),
        },
        "windows" => match parse_preflight_request::<MacControlWindowsRequest>(args, "windows") {
            Ok(request) => validate_windows_request(&request.clamped()),
            Err(error) => Some(error),
        },
        "act" => match parse_preflight_request::<MacControlActRequest>(args, "act") {
            Ok(request) => validate_act_request(&request.clamped()),
            Err(error) => Some(error),
        },
        "clipboard" => {
            match parse_preflight_request::<MacControlClipboardRequest>(args, "clipboard") {
                Ok(request) => validate_clipboard_request(&request.clamped()),
                Err(error) => Some(error),
            }
        }
        "menu" => {
            let request = match parse_preflight_request::<MacControlMenuRequest>(args, "menu") {
                Ok(request) => request.clamped(),
                Err(error) => return Some(error),
            };
            validate_menu_request(&request)
        }
        "dialog" => match parse_preflight_request::<MacControlDialogRequest>(args, "dialog") {
            Ok(request) => validate_dialog_request(&request.clamped()),
            Err(error) => Some(error),
        },
        _ => None,
    }
}

fn parse_preflight_request<T>(args: &serde_json::Value, label: &str) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value::<T>(args.clone())
        .map_err(|error| format!("Invalid mac_control {label} request: {error}"))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlSpacesOp {
    #[default]
    List,
    Switch,
    MoveWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlSpaceDirection {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlStringMatch {
    #[default]
    Exact,
    Contains,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlWindowsRequest {
    #[serde(default)]
    pub op: MacControlWindowsOp,
    #[serde(default)]
    pub window_scope: MacControlWindowsScope,
    #[serde(default)]
    pub target: MacControlTargetQuery,
    #[serde(default)]
    pub window_id: Option<String>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub width: Option<f64>,
    #[serde(default)]
    pub height: Option<f64>,
    #[serde(default = "default_snapshot_max_elements")]
    pub max_elements: usize,
    #[serde(default = "default_snapshot_max_depth")]
    pub max_depth: usize,
}

impl MacControlWindowsRequest {
    pub fn clamped(mut self) -> Self {
        self.target = self.target.normalized();
        self.window_id = normalize_optional_string(self.window_id);
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlWindowsOp {
    #[default]
    List,
    Focus,
    Move,
    Resize,
    Minimize,
    Close,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlWindowsScope {
    #[default]
    Frontmost,
    All,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlActRequest {
    #[serde(default)]
    pub op: MacControlActOp,
    #[serde(default)]
    pub target: MacControlTargetQuery,
    #[serde(default)]
    pub to_target: MacControlTargetQuery,
    #[serde(default)]
    pub ax_action: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub typing_profile: Option<MacControlTypingProfile>,
    #[serde(default)]
    pub typing_delay_ms: Option<u64>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub delta_x: Option<f64>,
    #[serde(default)]
    pub delta_y: Option<f64>,
    #[serde(default)]
    pub from_x: Option<f64>,
    #[serde(default)]
    pub from_y: Option<f64>,
    #[serde(default)]
    pub to_x: Option<f64>,
    #[serde(default)]
    pub to_y: Option<f64>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub steps: Option<usize>,
    #[serde(default)]
    pub motion_profile: Option<MacControlMotionProfile>,
    #[serde(default)]
    pub modifiers: Vec<String>,
    #[serde(default)]
    pub repeat: Option<usize>,
    #[serde(default)]
    pub interval_ms: Option<u64>,
    #[serde(default)]
    pub hold_ms: Option<u64>,
    #[serde(default)]
    pub include_snapshot: bool,
    #[serde(default = "default_snapshot_max_elements")]
    pub max_elements: usize,
    #[serde(default = "default_snapshot_max_depth")]
    pub max_depth: usize,
}

impl MacControlActRequest {
    pub fn clamped(mut self) -> Self {
        self.target = self.target.normalized();
        self.to_target = self.to_target.normalized();
        self.ax_action = normalize_optional_string(self.ax_action);
        self.text = normalize_optional_string(self.text);
        self.typing_delay_ms = self
            .typing_delay_ms
            .map(|delay_ms| delay_ms.min(HARD_TYPING_DELAY_MS));
        self.value = normalize_optional_string(self.value);
        self.key = normalize_optional_string(self.key);
        self.keys = self
            .keys
            .into_iter()
            .filter_map(|value| normalize_optional_string(Some(value)))
            .collect();
        self.modifiers = self
            .modifiers
            .into_iter()
            .filter_map(|value| normalize_optional_string(Some(value)))
            .collect();
        if self.max_elements == 0 {
            self.max_elements = DEFAULT_SNAPSHOT_MAX_ELEMENTS;
        }
        if self.max_depth == 0 {
            self.max_depth = DEFAULT_SNAPSHOT_MAX_DEPTH;
        }
        self.duration_ms = self
            .duration_ms
            .map(|duration_ms| duration_ms.min(HARD_MOTION_DURATION_MS));
        self.steps = self
            .steps
            .and_then(|steps| (steps > 0).then_some(steps.min(HARD_MOTION_STEPS)));
        self.repeat = self
            .repeat
            .and_then(|repeat| (repeat > 0).then_some(repeat.min(HARD_PRESS_REPEAT)));
        self.interval_ms = self
            .interval_ms
            .map(|interval_ms| interval_ms.min(HARD_PRESS_INTERVAL_MS));
        self.hold_ms = self.hold_ms.map(|hold_ms| hold_ms.min(HARD_PRESS_HOLD_MS));
        self.max_elements = self.max_elements.min(HARD_SNAPSHOT_MAX_ELEMENTS);
        self.max_depth = self.max_depth.min(HARD_SNAPSHOT_MAX_DEPTH);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlTypingProfile {
    Instant,
    Steady,
    Human,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlMotionProfile {
    Linear,
    Human,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlActOp {
    #[default]
    Click,
    DryRun,
    PerformAction,
    ClickPoint,
    MoveCursor,
    DoubleClick,
    RightClick,
    Type,
    Paste,
    SetValue,
    Hotkey,
    Press,
    Scroll,
    Drag,
    Swipe,
}

pub fn normalize_perform_ax_action(action: &str) -> Option<String> {
    let action = action.trim();
    if action.is_empty() {
        return None;
    }
    if action.len() > 128
        || !action
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return None;
    }
    let canonical = match action.to_ascii_lowercase().as_str() {
        "press" | "axpress" => Some("AXPress"),
        "show_menu" | "showmenu" | "axshowmenu" => Some("AXShowMenu"),
        "confirm" | "axconfirm" => Some("AXConfirm"),
        "cancel" | "axcancel" => Some("AXCancel"),
        "increment" | "axincrement" => Some("AXIncrement"),
        "decrement" | "axdecrement" => Some("AXDecrement"),
        "pick" | "axpick" => Some("AXPick"),
        "raise" | "axraise" => Some("AXRaise"),
        "show_default_ui" | "showdefaultui" | "axshowdefaultui" => Some("AXShowDefaultUI"),
        "show_alternate_ui" | "showalternateui" | "axshowalternateui" => Some("AXShowAlternateUI"),
        _ => None,
    };
    Some(canonical.unwrap_or(action).to_string())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlMenuRequest {
    #[serde(default)]
    pub op: MacControlMenuOp,
    #[serde(default)]
    pub scope: MacControlMenuScope,
    #[serde(default)]
    pub path: Vec<String>,
    #[serde(default)]
    pub menu_index: Option<usize>,
    #[serde(default)]
    pub verify: bool,
    #[serde(default = "default_menu_max_depth")]
    pub max_depth: usize,
    #[serde(default)]
    pub app_hint: Option<String>,
    #[serde(default = "default_menu_include_ocr")]
    pub include_ocr: bool,
    #[serde(default = "default_menu_popover_limit")]
    pub limit: usize,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub min_confidence: Option<f32>,
    #[serde(default)]
    pub recognition_level: MacControlOcrRecognitionLevel,
}

impl Default for MacControlMenuRequest {
    fn default() -> Self {
        Self {
            op: MacControlMenuOp::default(),
            scope: MacControlMenuScope::default(),
            path: Vec::new(),
            menu_index: None,
            verify: false,
            max_depth: default_menu_max_depth(),
            app_hint: None,
            include_ocr: default_menu_include_ocr(),
            limit: default_menu_popover_limit(),
            languages: Vec::new(),
            min_confidence: None,
            recognition_level: MacControlOcrRecognitionLevel::default(),
        }
    }
}

impl MacControlMenuRequest {
    pub fn clamped(mut self) -> Self {
        self.path = self
            .path
            .into_iter()
            .filter_map(|value| normalize_optional_string(Some(value)))
            .collect();
        self.app_hint = normalize_optional_string(self.app_hint);
        self.menu_index = self.menu_index.filter(|index| *index < 10_000);
        self.languages = self
            .languages
            .into_iter()
            .filter_map(|language| normalize_optional_string(Some(language)))
            .take(16)
            .collect();
        self.min_confidence = self
            .min_confidence
            .filter(|value| value.is_finite())
            .map(|value| value.clamp(0.0, 1.0));
        if self.max_depth == 0 {
            self.max_depth = default_menu_max_depth();
        }
        if self.limit == 0 {
            self.limit = default_menu_popover_limit();
        }
        self.max_depth = self.max_depth.min(8);
        self.limit = self.limit.min(HARD_MENU_POPOVER_LIMIT);
        self
    }
}

fn default_menu_max_depth() -> usize {
    3
}

fn default_menu_include_ocr() -> bool {
    true
}

fn default_menu_popover_limit() -> usize {
    DEFAULT_MENU_POPOVER_LIMIT
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlMenuOp {
    #[default]
    List,
    Click,
    Popover,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlMenuScope {
    #[default]
    App,
    System,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlClipboardRequest {
    #[serde(default)]
    pub op: MacControlClipboardOp,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default = "default_clipboard_max_chars")]
    pub max_chars: usize,
    #[serde(skip)]
    pub text_original_len: Option<usize>,
    #[serde(skip)]
    pub text_truncated: bool,
}

impl MacControlClipboardRequest {
    pub fn clamped(mut self) -> Self {
        if self.max_chars == 0 {
            self.max_chars = DEFAULT_CLIPBOARD_MAX_CHARS;
        }
        self.max_chars = self.max_chars.min(HARD_CLIPBOARD_MAX_CHARS);
        if let Some(text) = self.text.as_mut() {
            if self.text_original_len.is_none() {
                let original_len = text.chars().count();
                self.text_original_len = Some(original_len);
                self.text_truncated = original_len > HARD_CLIPBOARD_SET_CHARS;
            }
            if self.text_truncated {
                *text = text.chars().take(HARD_CLIPBOARD_SET_CHARS).collect();
            }
        }
        self
    }
}

fn default_clipboard_max_chars() -> usize {
    DEFAULT_CLIPBOARD_MAX_CHARS
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlClipboardOp {
    #[default]
    Get,
    Set,
    Clear,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlDialogRequest {
    #[serde(default)]
    pub op: MacControlDialogOp,
    #[serde(default)]
    pub target: MacControlTargetQuery,
    #[serde(default, alias = "button")]
    pub button_text: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub field: Option<String>,
    #[serde(default, alias = "field_index")]
    pub field_index: Option<usize>,
    #[serde(default)]
    pub clear: bool,
    #[serde(default, alias = "path")]
    pub file_path: Option<String>,
    #[serde(default, alias = "name")]
    pub file_name: Option<String>,
    #[serde(default, alias = "select")]
    pub select_button: Option<String>,
    #[serde(default, alias = "ensure_expanded")]
    pub ensure_expanded: bool,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub include_snapshot: bool,
    #[serde(default = "default_snapshot_max_elements")]
    pub max_elements: usize,
    #[serde(default = "default_snapshot_max_depth")]
    pub max_depth: usize,
}

impl MacControlDialogRequest {
    pub fn clamped(mut self) -> Self {
        self.target = self.target.normalized();
        self.button_text = normalize_optional_string(self.button_text);
        self.text = normalize_optional_string(self.text);
        self.field = normalize_optional_string(self.field);
        self.file_path = normalize_optional_string(self.file_path);
        self.file_name = normalize_optional_string(self.file_name);
        self.select_button = normalize_optional_string(self.select_button);
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlDialogOp {
    #[default]
    Inspect,
    List,
    Click,
    Input,
    File,
    Accept,
    Dismiss,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlWaitRequest {
    #[serde(default)]
    pub op: MacControlWaitOp,
    #[serde(default)]
    pub target: MacControlTargetQuery,
    #[serde(default)]
    pub include_snapshot: bool,
    #[serde(default = "default_wait_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_wait_poll_ms")]
    pub poll_ms: u64,
    #[serde(default = "default_snapshot_max_elements")]
    pub max_elements: usize,
    #[serde(default = "default_snapshot_max_depth")]
    pub max_depth: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlWaitOp {
    #[default]
    Present,
    Gone,
}

impl MacControlWaitRequest {
    pub fn clamped(mut self) -> Self {
        self.target = self.target.normalized();
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
    pub window_title_match: MacControlStringMatch,
    #[serde(default)]
    pub element_id: Option<String>,
    #[serde(default)]
    pub snapshot_id: Option<String>,
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
    fn normalized(mut self) -> Self {
        self.app_name = normalize_optional_string(self.app_name);
        self.bundle_id = normalize_optional_string(self.bundle_id);
        self.window_title = normalize_optional_string(self.window_title);
        self.element_id = normalize_optional_string(self.element_id);
        self.snapshot_id = normalize_optional_string(self.snapshot_id);
        self.text = normalize_optional_string(self.text);
        self.role = normalize_optional_string(self.role);
        if self.enabled == Some(false) {
            self.enabled = None;
        }
        if self.focused == Some(false) {
            self.focused = None;
        }
        self
    }

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

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
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

pub async fn diagnostics(request: MacControlDiagnosticsRequest) -> MacControlDiagnosticsResponse {
    let request = request.clamped();
    let status = status().await;
    let mut result = MacControlDiagnosticsResult {
        op: request.op,
        generated_at: Utc::now().to_rfc3339(),
        snapshot_cache: snapshot_cache_summaries(request.limit),
        recent_errors: runtime_stats().recent_errors,
        focus_anchor: capture_focus_anchor().await,
        export_path: None,
        warnings: Vec::new(),
    };

    if request.op == MacControlDiagnosticsOp::Export {
        match create_diagnostics_export_path() {
            Ok(path) => {
                result.export_path = Some(path.display().to_string());
                if let Err(error) = write_diagnostics_bundle(&path, &status, &result) {
                    result
                        .warnings
                        .push(format!("mac_control diagnostics export failed: {error}"));
                    return MacControlDiagnosticsResponse {
                        status,
                        result: Some(result),
                        error: Some(error),
                    };
                }
            }
            Err(error) => {
                result
                    .warnings
                    .push(format!("mac_control diagnostics export failed: {error}"));
                return MacControlDiagnosticsResponse {
                    status,
                    result: Some(result),
                    error: Some(error),
                };
            }
        }
    }

    MacControlDiagnosticsResponse {
        status,
        result: Some(result),
        error: None,
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
        Err(error) => {
            record_error("snapshot", &error);
            MacControlSnapshotResponse {
                status,
                snapshot: None,
                error: Some(error),
            }
        }
    }
}

pub async fn elements(request: MacControlElementsRequest) -> MacControlElementsResponse {
    let request = request.clamped();
    let Some(bridge) = available_bridge() else {
        return unsupported_elements_response(unsupported_reason());
    };
    let system_permissions = bridge.system_permissions().await;
    let status = status_from_system_permissions(true, true, system_permissions.clone());
    if !system_permissions.supported {
        return MacControlElementsResponse {
            status,
            result: None,
            error: Some("macOS control is unsupported in this runtime.".to_string()),
        };
    }
    if !permission_granted(&system_permissions, "accessibility") {
        return MacControlElementsResponse {
            status,
            result: None,
            error: Some("mac_control elements.find requires Accessibility permission.".to_string()),
        };
    }

    match bridge.elements(request).await {
        Ok(result) => MacControlElementsResponse {
            status,
            result: Some(result),
            error: None,
        },
        Err(error) => {
            record_error("elements", &error);
            MacControlElementsResponse {
                status,
                result: None,
                error: Some(error),
            }
        }
    }
}

pub async fn wait(request: MacControlWaitRequest) -> MacControlWaitResponse {
    let request = request.clamped();
    let target = request.target.clone();
    let op = request.op;
    let Some(bridge) = available_bridge() else {
        return unsupported_wait_response(unsupported_reason(), op, target);
    };
    let system_permissions = bridge.system_permissions().await;
    let status = status_from_system_permissions(true, true, system_permissions.clone());
    if target.is_empty() {
        return MacControlWaitResponse {
            status,
            op,
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
            op,
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
            op,
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
                ..Default::default()
            })
            .await
        {
            Ok(snapshot) => {
                let matches = find_target_matches(&snapshot, &target);
                let target_matched = target_matches(&target, &matches);
                let matched = wait_condition_satisfied(op, target_matched);
                record_snapshot(snapshot.clone());
                if matched {
                    return MacControlWaitResponse {
                        status,
                        op,
                        matched: true,
                        elapsed_ms: elapsed_ms(started),
                        attempts,
                        target,
                        matches,
                        snapshot: request.include_snapshot.then_some(snapshot),
                        error: None,
                    };
                }
                last_matches = matches;
                if request.include_snapshot {
                    last_snapshot = Some(snapshot);
                }
            }
            Err(error) => {
                record_error("wait.snapshot", &error);
                last_error = Some(error);
            }
        }

        if started.elapsed() >= timeout {
            return MacControlWaitResponse {
                status,
                op,
                matched: false,
                elapsed_ms: elapsed_ms(started),
                attempts,
                target,
                matches: last_matches,
                snapshot: last_snapshot,
                error: Some(
                    last_error.unwrap_or_else(|| timeout_wait_message(op, request.timeout_ms)),
                ),
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
    if matches!(
        request.op,
        MacControlAppsOp::Activate | MacControlAppsOp::Quit
    ) && !apps_request_has_target(&request)
    {
        return MacControlAppsResponse {
            status,
            result: None,
            error: Some(format!(
                "mac_control apps.{} requires one of pid, bundleId, or appName.",
                apps_op_name(request.op)
            )),
        };
    }
    if request.op == MacControlAppsOp::Launch && !apps_launch_request_has_target(&request) {
        return MacControlAppsResponse {
            status,
            result: None,
            error: Some("mac_control apps.launch requires bundleId or appName.".to_string()),
        };
    }

    match bridge.apps(request).await {
        Ok(result) => MacControlAppsResponse {
            status,
            result: Some(result),
            error: None,
        },
        Err(error) => {
            record_error("apps", &error);
            MacControlAppsResponse {
                status,
                result: None,
                error: Some(error),
            }
        }
    }
}

pub async fn dock(request: MacControlDockRequest) -> MacControlDockResponse {
    let request = request.clamped();
    let Some(bridge) = available_bridge() else {
        return unsupported_dock_response(unsupported_reason());
    };
    let system_permissions = bridge.system_permissions().await;
    let status = status_from_system_permissions(true, true, system_permissions.clone());
    if !system_permissions.supported {
        return MacControlDockResponse {
            status,
            result: None,
            error: Some("macOS control is unsupported in this runtime.".to_string()),
        };
    }
    if let Some(error) = validate_dock_request(&request) {
        return MacControlDockResponse {
            status,
            result: None,
            error: Some(error),
        };
    }

    match bridge.dock(request).await {
        Ok(result) => MacControlDockResponse {
            status,
            result: Some(result),
            error: None,
        },
        Err(error) => {
            record_error("dock", &error);
            MacControlDockResponse {
                status,
                result: None,
                error: Some(error),
            }
        }
    }
}

pub async fn spaces(request: MacControlSpacesRequest) -> MacControlSpacesResponse {
    let request = request.clamped();
    let Some(bridge) = available_bridge() else {
        return unsupported_spaces_response(unsupported_reason());
    };
    let system_permissions = bridge.system_permissions().await;
    let status = status_from_system_permissions(true, true, system_permissions.clone());
    if !system_permissions.supported {
        return MacControlSpacesResponse {
            status,
            result: None,
            error: Some("macOS control is unsupported in this runtime.".to_string()),
        };
    }
    if request.op != MacControlSpacesOp::List
        && !permission_granted(&system_permissions, "accessibility")
    {
        return MacControlSpacesResponse {
            status,
            result: None,
            error: Some(
                "mac_control spaces mutation requires Accessibility permission.".to_string(),
            ),
        };
    }
    if let Some(error) = validate_spaces_request(&request) {
        return MacControlSpacesResponse {
            status,
            result: None,
            error: Some(error),
        };
    }

    match bridge.spaces(request).await {
        Ok(result) => MacControlSpacesResponse {
            status,
            result: Some(result),
            error: None,
        },
        Err(error) => {
            record_error("spaces", &error);
            MacControlSpacesResponse {
                status,
                result: None,
                error: Some(error),
            }
        }
    }
}

pub async fn capture_focus_anchor() -> Option<MacControlFocusAnchor> {
    let response = apps(MacControlAppsRequest {
        op: MacControlAppsOp::Frontmost,
        limit: 1,
        ..Default::default()
    })
    .await;
    let focused_window = capture_focus_anchor_window().await;
    let focused_window_id = focused_window.as_ref().map(|window| window.id.clone());
    let focused_window_title = focused_window
        .as_ref()
        .and_then(|window| window.title.clone());
    response.result?.frontmost.map(|app| MacControlFocusAnchor {
        pid: app.pid,
        bundle_id: app.bundle_id,
        name: app.name,
        focused_window_id,
        focused_window_title,
    })
}

pub async fn restore_focus_anchor(anchor: &MacControlFocusAnchor) -> Result<(), String> {
    let mut errors = Vec::new();
    let mut app_restored = false;
    for request in focus_anchor_activate_requests(anchor) {
        let response = apps(request).await;
        if let Some(result) = response.result {
            if result.activated.is_some() {
                app_restored = true;
                break;
            }
        }
        if let Some(error) = response.error {
            errors.push(error);
        }
    }

    if !app_restored {
        return if errors.is_empty() {
            Err("mac_control approval focus restore did not activate any app.".to_string())
        } else {
            Err(format!(
                "mac_control approval focus restore failed: {}",
                errors.join("; ")
            ))
        };
    }

    if let Err(error) = restore_focus_anchor_window(anchor).await {
        app_warn!(
            "mac_control",
            "approval_focus",
            "Failed to restore focused macOS window after approval: {}",
            error
        );
    }

    Ok(())
}

async fn capture_focus_anchor_window() -> Option<MacControlWindowSummary> {
    let response = windows(MacControlWindowsRequest {
        op: MacControlWindowsOp::List,
        window_scope: MacControlWindowsScope::Frontmost,
        max_elements: 20,
        max_depth: 2,
        ..Default::default()
    })
    .await;
    let result = response.result?;
    result
        .windows
        .iter()
        .find(|window| window.focused)
        .cloned()
        .or_else(|| result.windows.into_iter().next())
}

async fn restore_focus_anchor_window(anchor: &MacControlFocusAnchor) -> Result<(), String> {
    let requests = focus_anchor_window_requests(anchor);
    if requests.is_empty() {
        return Ok(());
    }

    let mut errors = Vec::new();
    for request in requests {
        let response = windows(request).await;
        if let Some(result) = response.result {
            if result.verification.as_ref().is_some_and(|verification| {
                verification.status == MacControlVerificationStatus::Verified
            }) || result
                .acted_window
                .as_ref()
                .is_some_and(|window| window.focused)
            {
                return Ok(());
            }
            if let Some(verification) = result.verification {
                errors.push(verification.summary);
            }
        }
        if let Some(error) = response.error {
            errors.push(error);
        }
    }

    if errors.is_empty() {
        Err("focused window restore did not match any previous window anchor.".to_string())
    } else {
        Err(format!(
            "focused window restore failed: {}",
            errors.join("; ")
        ))
    }
}

fn focus_anchor_activate_requests(anchor: &MacControlFocusAnchor) -> Vec<MacControlAppsRequest> {
    let mut requests = Vec::new();
    if anchor.pid > 0 {
        requests.push(MacControlAppsRequest {
            op: MacControlAppsOp::Activate,
            pid: Some(anchor.pid),
            limit: 1,
            ..Default::default()
        });
    }
    if let Some(bundle_id) = anchor.bundle_id.as_ref().filter(|value| !value.is_empty()) {
        requests.push(MacControlAppsRequest {
            op: MacControlAppsOp::Activate,
            bundle_id: Some(bundle_id.clone()),
            limit: 1,
            ..Default::default()
        });
    }
    if let Some(name) = anchor.name.as_ref().filter(|value| !value.is_empty()) {
        requests.push(MacControlAppsRequest {
            op: MacControlAppsOp::Activate,
            app_name: Some(name.clone()),
            limit: 1,
            ..Default::default()
        });
    }
    requests
}

fn focus_anchor_window_requests(anchor: &MacControlFocusAnchor) -> Vec<MacControlWindowsRequest> {
    let mut requests = Vec::new();
    if let Some(window_id) = focus_anchor_scoped_window_id(anchor) {
        requests.push(MacControlWindowsRequest {
            op: MacControlWindowsOp::Focus,
            window_scope: MacControlWindowsScope::All,
            window_id: Some(window_id),
            target: MacControlTargetQuery {
                window_title: anchor.focused_window_title.clone(),
                window_title_match: MacControlStringMatch::Exact,
                ..Default::default()
            },
            max_elements: 20,
            max_depth: 2,
            ..Default::default()
        });
    }
    if let Some(title) = anchor
        .focused_window_title
        .as_ref()
        .filter(|value| !value.is_empty())
    {
        requests.push(MacControlWindowsRequest {
            op: MacControlWindowsOp::Focus,
            window_scope: MacControlWindowsScope::Frontmost,
            target: MacControlTargetQuery {
                window_title: Some(title.clone()),
                window_title_match: MacControlStringMatch::Exact,
                ..Default::default()
            },
            max_elements: 20,
            max_depth: 2,
            ..Default::default()
        });
    }
    requests
}

fn focus_anchor_scoped_window_id(anchor: &MacControlFocusAnchor) -> Option<String> {
    let window_id = anchor
        .focused_window_id
        .as_ref()
        .filter(|value| !value.is_empty())?;
    if let Some(index) = window_id
        .strip_prefix("win_")
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|index| *index > 0)
    {
        return Some(format!("win_{}_{}", anchor.pid, index));
    }
    Some(window_id.clone())
}

pub async fn windows(request: MacControlWindowsRequest) -> MacControlWindowsResponse {
    let request = request.clamped();
    let Some(bridge) = available_bridge() else {
        return unsupported_windows_response(unsupported_reason());
    };
    let system_permissions = bridge.system_permissions().await;
    let status = status_from_system_permissions(true, true, system_permissions.clone());
    if !system_permissions.supported {
        return MacControlWindowsResponse {
            status,
            result: None,
            error: Some("macOS control is unsupported in this runtime.".to_string()),
        };
    }
    if !permission_granted(&system_permissions, "accessibility") {
        return MacControlWindowsResponse {
            status,
            result: None,
            error: Some("mac_control windows requires Accessibility permission.".to_string()),
        };
    }
    if request.op != MacControlWindowsOp::List && !windows_request_has_target(&request) {
        return MacControlWindowsResponse {
            status,
            result: None,
            error: Some(
                "mac_control windows operation requires windowId or target.windowTitle."
                    .to_string(),
            ),
        };
    }
    if let Some(error) = validate_windows_request(&request) {
        return MacControlWindowsResponse {
            status,
            result: None,
            error: Some(error),
        };
    }

    match bridge.windows(request).await {
        Ok(result) => MacControlWindowsResponse {
            status,
            result: Some(result),
            error: None,
        },
        Err(error) => {
            record_error("windows", &error);
            MacControlWindowsResponse {
                status,
                result: None,
                error: Some(error),
            }
        }
    }
}

pub async fn act(request: MacControlActRequest) -> MacControlActResponse {
    let request = request.clamped();
    let Some(bridge) = available_bridge() else {
        return unsupported_act_response(unsupported_reason());
    };
    let system_permissions = bridge.system_permissions().await;
    let status = status_from_system_permissions(true, true, system_permissions.clone());
    if !system_permissions.supported {
        return MacControlActResponse {
            status,
            result: None,
            error: Some("macOS control is unsupported in this runtime.".to_string()),
        };
    }
    if !permission_granted(&system_permissions, "accessibility") {
        return MacControlActResponse {
            status,
            result: None,
            error: Some("mac_control act requires Accessibility permission.".to_string()),
        };
    }
    if let Some(error) = validate_act_request(&request) {
        return MacControlActResponse {
            status,
            result: None,
            error: Some(error),
        };
    }

    match bridge.act(request).await {
        Ok(result) => MacControlActResponse {
            status,
            result: Some(result),
            error: None,
        },
        Err(error) => {
            record_error("act", &error);
            MacControlActResponse {
                status,
                result: None,
                error: Some(error),
            }
        }
    }
}

pub async fn menu(request: MacControlMenuRequest) -> MacControlMenuResponse {
    let request = request.clamped();
    let Some(bridge) = available_bridge() else {
        return unsupported_menu_response(unsupported_reason());
    };
    let system_permissions = bridge.system_permissions().await;
    let status = status_from_system_permissions(true, true, system_permissions.clone());
    if !system_permissions.supported {
        return MacControlMenuResponse {
            status,
            result: None,
            error: Some("macOS control is unsupported in this runtime.".to_string()),
        };
    }
    if !permission_granted(&system_permissions, "accessibility") {
        return MacControlMenuResponse {
            status,
            result: None,
            error: Some("mac_control menu requires Accessibility permission.".to_string()),
        };
    }
    if let Some(error) = validate_menu_request(&request) {
        return MacControlMenuResponse {
            status,
            result: None,
            error: Some(error),
        };
    }

    match bridge.menu(request).await {
        Ok(result) => MacControlMenuResponse {
            status,
            result: Some(result),
            error: None,
        },
        Err(error) => {
            record_error("menu", &error);
            MacControlMenuResponse {
                status,
                result: None,
                error: Some(error),
            }
        }
    }
}

pub async fn clipboard(request: MacControlClipboardRequest) -> MacControlClipboardResponse {
    let request = request.clamped();
    let Some(bridge) = available_bridge() else {
        return unsupported_clipboard_response(unsupported_reason());
    };
    let system_permissions = bridge.system_permissions().await;
    let status = status_from_system_permissions(true, true, system_permissions.clone());
    if !system_permissions.supported {
        return MacControlClipboardResponse {
            status,
            result: None,
            error: Some("macOS control is unsupported in this runtime.".to_string()),
        };
    }
    if let Some(error) = validate_clipboard_request(&request) {
        return MacControlClipboardResponse {
            status,
            result: None,
            error: Some(error),
        };
    }

    match bridge.clipboard(request).await {
        Ok(result) => MacControlClipboardResponse {
            status,
            result: Some(result),
            error: None,
        },
        Err(error) => {
            record_error("clipboard", &error);
            MacControlClipboardResponse {
                status,
                result: None,
                error: Some(error),
            }
        }
    }
}

pub async fn dialog(request: MacControlDialogRequest) -> MacControlDialogResponse {
    let request = request.clamped();
    let Some(bridge) = available_bridge() else {
        return unsupported_dialog_response(unsupported_reason());
    };
    let system_permissions = bridge.system_permissions().await;
    let status = status_from_system_permissions(true, true, system_permissions.clone());
    if !system_permissions.supported {
        return MacControlDialogResponse {
            status,
            result: None,
            error: Some("macOS control is unsupported in this runtime.".to_string()),
        };
    }
    if !permission_granted(&system_permissions, "accessibility") {
        return MacControlDialogResponse {
            status,
            result: None,
            error: Some("mac_control dialog requires Accessibility permission.".to_string()),
        };
    }

    match bridge.dialog(request).await {
        Ok(result) => MacControlDialogResponse {
            status,
            result: Some(result),
            error: None,
        },
        Err(error) => {
            record_error("dialog", &error);
            MacControlDialogResponse {
                status,
                result: None,
                error: Some(error),
            }
        }
    }
}

pub async fn visual(request: MacControlVisualRequest) -> MacControlVisualResponse {
    let request = request.clamped();
    match request.op {
        MacControlVisualOp::Observe => visual_observe(request).await,
        MacControlVisualOp::Point => visual_point(request).await,
        MacControlVisualOp::Ocr => visual_ocr(request).await,
        MacControlVisualOp::FindText => visual_find_text(request).await,
    }
}

async fn visual_observe(request: MacControlVisualRequest) -> MacControlVisualResponse {
    let annotate = request.annotate;
    let ui_map_limit = request.ui_map_limit;
    let response = snapshot(MacControlSnapshotRequest {
        include_screenshot: true,
        screenshot_target: request.screenshot_target,
        display_id: request.display_id,
        window_id: request.window_id,
        max_elements: request.max_elements,
        max_depth: request.max_depth,
    })
    .await;

    let Some(snapshot) = response.snapshot else {
        return MacControlVisualResponse {
            status: response.status,
            result: None,
            error: response.error,
        };
    };

    let result = visual_observe_result(snapshot, annotate, ui_map_limit);
    let error = if result.screenshot.is_none() {
        Some(
            "mac_control visual.observe requires a screenshot; check Screen Recording permission or screenshot target metadata."
                .to_string(),
        )
    } else {
        response.error
    };

    MacControlVisualResponse {
        status: response.status,
        result: Some(result),
        error,
    }
}

async fn visual_point(request: MacControlVisualRequest) -> MacControlVisualResponse {
    let Some(bridge) = available_bridge() else {
        return unsupported_visual_response(unsupported_reason());
    };
    let system_permissions = bridge.system_permissions().await;
    let status = status_from_system_permissions(true, true, system_permissions.clone());
    if !system_permissions.supported {
        return MacControlVisualResponse {
            status,
            result: None,
            error: Some("macOS control is unsupported in this runtime.".to_string()),
        };
    }
    if !permission_granted(&system_permissions, "accessibility") {
        return MacControlVisualResponse {
            status,
            result: None,
            error: Some("mac_control visual.point requires Accessibility permission.".to_string()),
        };
    }
    if !permission_granted(&system_permissions, "screen_recording") {
        return MacControlVisualResponse {
            status,
            result: None,
            error: Some(
                "mac_control visual.point requires Screen Recording permission.".to_string(),
            ),
        };
    }

    let Some(snapshot_id) = request.snapshot_id.as_deref() else {
        return MacControlVisualResponse {
            status,
            result: None,
            error: Some(
                "mac_control visual.point requires snapshotId from visual.observe or snapshot."
                    .to_string(),
            ),
        };
    };
    let Some(raw_x) = request.x else {
        return MacControlVisualResponse {
            status,
            result: None,
            error: Some("mac_control visual.point requires x.".to_string()),
        };
    };
    let Some(raw_y) = request.y else {
        return MacControlVisualResponse {
            status,
            result: None,
            error: Some("mac_control visual.point requires y.".to_string()),
        };
    };
    if !raw_x.is_finite() || !raw_y.is_finite() {
        return MacControlVisualResponse {
            status,
            result: None,
            error: Some("mac_control visual.point requires finite x and y values.".to_string()),
        };
    }

    let Some(snapshot) = cached_snapshot(snapshot_id) else {
        return MacControlVisualResponse {
            status,
            result: None,
            error: Some(format!(
                "mac_control visual.point snapshotId '{snapshot_id}' was not found or expired; call visual.observe again."
            )),
        };
    };

    match resolve_visual_point(
        &snapshot,
        request.coordinate_space,
        raw_x,
        raw_y,
        request.limit,
    ) {
        Ok(result) => MacControlVisualResponse {
            status,
            result: Some(result),
            error: None,
        },
        Err(error) => MacControlVisualResponse {
            status,
            result: None,
            error: Some(error),
        },
    }
}

async fn visual_ocr(request: MacControlVisualRequest) -> MacControlVisualResponse {
    visual_ocr_or_find_text(request, false).await
}

async fn visual_find_text(request: MacControlVisualRequest) -> MacControlVisualResponse {
    visual_ocr_or_find_text(request, true).await
}

async fn visual_ocr_or_find_text(
    request: MacControlVisualRequest,
    find_text: bool,
) -> MacControlVisualResponse {
    let Some(bridge) = available_bridge() else {
        return unsupported_visual_response(unsupported_reason());
    };
    let system_permissions = bridge.system_permissions().await;
    let status = status_from_system_permissions(true, true, system_permissions.clone());
    if !system_permissions.supported {
        return MacControlVisualResponse {
            status,
            result: None,
            error: Some("macOS control is unsupported in this runtime.".to_string()),
        };
    }
    if !permission_granted(&system_permissions, "screen_recording") {
        return MacControlVisualResponse {
            status,
            result: None,
            error: Some("mac_control visual OCR requires Screen Recording permission.".to_string()),
        };
    }
    if find_text && !permission_granted(&system_permissions, "accessibility") {
        return MacControlVisualResponse {
            status,
            result: None,
            error: Some(
                "mac_control visual.find_text requires Accessibility permission.".to_string(),
            ),
        };
    }
    if request.snapshot_id.is_none() && !permission_granted(&system_permissions, "accessibility") {
        return MacControlVisualResponse {
            status,
            result: None,
            error: Some(
                "mac_control visual OCR without snapshotId requires Accessibility permission to capture a fresh snapshot."
                    .to_string(),
            ),
        };
    }
    if find_text && request.text.is_none() {
        return MacControlVisualResponse {
            status,
            result: None,
            error: Some("mac_control visual.find_text requires text.".to_string()),
        };
    }

    let snapshot = match visual_snapshot_for_ocr(&*bridge, &request).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            record_error("visual", &error);
            return MacControlVisualResponse {
                status,
                result: None,
                error: Some(error),
            };
        }
    };
    let screenshot = match snapshot.screenshot.clone() {
        Some(screenshot) => screenshot,
        None => {
            return MacControlVisualResponse {
                status,
                result: None,
                error: Some(format!(
                    "mac_control visual OCR snapshotId '{}' has no screenshot metadata; call visual.observe or omit snapshotId.",
                    snapshot.snapshot_id
                )),
            };
        }
    };

    let raw_blocks = match bridge
        .ocr(MacControlOcrRequest {
            screenshot: screenshot.clone(),
            languages: request.languages.clone(),
            recognition_level: request.recognition_level,
        })
        .await
    {
        Ok(blocks) => blocks,
        Err(error) => {
            record_error("visual.ocr", &error);
            return MacControlVisualResponse {
                status,
                result: None,
                error: Some(error),
            };
        }
    };

    let text_blocks = match resolve_ocr_text_blocks(&snapshot, raw_blocks, request.min_confidence) {
        Ok(blocks) => blocks,
        Err(error) => {
            return MacControlVisualResponse {
                status,
                result: None,
                error: Some(error),
            };
        }
    };
    let text_matches = if find_text {
        resolve_ocr_text_matches(
            &snapshot,
            &text_blocks,
            request.text.as_deref().unwrap_or_default(),
            request.text_match,
            request.limit,
        )
    } else {
        Vec::new()
    };
    let suggested_action = text_matches
        .first()
        .and_then(|candidate| candidate.suggested_action.clone());

    MacControlVisualResponse {
        status,
        result: Some(MacControlVisualResult {
            op: if find_text {
                MacControlVisualOp::FindText
            } else {
                MacControlVisualOp::Ocr
            },
            snapshot_id: Some(snapshot.snapshot_id),
            snapshot: None,
            screenshot: Some(screenshot),
            annotated_screenshot: None,
            ui_map: Vec::new(),
            coordinate_space: None,
            image_point: None,
            screen_point: None,
            inside_frame: None,
            hit_elements: Vec::new(),
            nearest_elements: Vec::new(),
            text_blocks,
            text_matches,
            suggested_action,
            warnings: snapshot.warnings,
        }),
        error: None,
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
        Err(error) => {
            record_error("capture_frame", &error);
            MacControlFrameResponse {
                status,
                frame: None,
                error: Some(error),
            }
        }
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
        target: MacControlScreenshotTarget::Display,
        display_id: None,
        window_id: None,
        window_title: None,
        bounds_points: None,
        scale: None,
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
        stats: runtime_stats(),
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

pub fn unsupported_elements_response(message: &str) -> MacControlElementsResponse {
    MacControlElementsResponse {
        status: unsupported_status(message),
        result: None,
        error: Some(message.to_string()),
    }
}

pub fn unsupported_wait_response(
    message: &str,
    op: MacControlWaitOp,
    target: MacControlTargetQuery,
) -> MacControlWaitResponse {
    MacControlWaitResponse {
        status: unsupported_status(message),
        op,
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

pub fn unsupported_dock_response(message: &str) -> MacControlDockResponse {
    MacControlDockResponse {
        status: unsupported_status(message),
        result: None,
        error: Some(message.to_string()),
    }
}

pub fn unsupported_spaces_response(message: &str) -> MacControlSpacesResponse {
    MacControlSpacesResponse {
        status: unsupported_status(message),
        result: None,
        error: Some(message.to_string()),
    }
}

pub fn unsupported_windows_response(message: &str) -> MacControlWindowsResponse {
    MacControlWindowsResponse {
        status: unsupported_status(message),
        result: None,
        error: Some(message.to_string()),
    }
}

pub fn unsupported_act_response(message: &str) -> MacControlActResponse {
    MacControlActResponse {
        status: unsupported_status(message),
        result: None,
        error: Some(message.to_string()),
    }
}

pub fn unsupported_menu_response(message: &str) -> MacControlMenuResponse {
    MacControlMenuResponse {
        status: unsupported_status(message),
        result: None,
        error: Some(message.to_string()),
    }
}

pub fn unsupported_clipboard_response(message: &str) -> MacControlClipboardResponse {
    MacControlClipboardResponse {
        status: unsupported_status(message),
        result: None,
        error: Some(message.to_string()),
    }
}

pub fn unsupported_dialog_response(message: &str) -> MacControlDialogResponse {
    MacControlDialogResponse {
        status: unsupported_status(message),
        result: None,
        error: Some(message.to_string()),
    }
}

pub fn unsupported_visual_response(message: &str) -> MacControlVisualResponse {
    MacControlVisualResponse {
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

fn apps_request_has_target(request: &MacControlAppsRequest) -> bool {
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

fn apps_launch_request_has_target(request: &MacControlAppsRequest) -> bool {
    request
        .bundle_id
        .as_deref()
        .is_some_and(|value| !value.is_empty())
        || request
            .app_name
            .as_deref()
            .is_some_and(|value| !value.is_empty())
}

fn dock_request_has_target(request: &MacControlDockRequest) -> bool {
    request
        .dock_item_id
        .as_deref()
        .is_some_and(|value| !value.is_empty())
        || request
            .bundle_id
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        || request
            .app_name
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        || request
            .item_path
            .as_deref()
            .is_some_and(|value| !value.is_empty())
}

fn validate_dock_request(request: &MacControlDockRequest) -> Option<String> {
    match request.op {
        MacControlDockOp::Launch | MacControlDockOp::Menu | MacControlDockOp::SelectMenu => {
            if !dock_request_has_target(request) {
                return Some(format!(
                    "mac_control dock.{} requires dockItemId, bundleId, appName, or itemPath.",
                    dock_op_label(request.op)
                ));
            }
        }
        MacControlDockOp::List | MacControlDockOp::Hide | MacControlDockOp::Show => {}
    }
    if request.op == MacControlDockOp::SelectMenu
        && request.menu_item.is_none()
        && request.menu_index.is_none()
    {
        return Some("mac_control dock.select_menu requires menuItem or menuIndex.".to_string());
    }
    None
}

fn dock_op_label(op: MacControlDockOp) -> &'static str {
    match op {
        MacControlDockOp::List => "list",
        MacControlDockOp::Launch => "launch",
        MacControlDockOp::Hide => "hide",
        MacControlDockOp::Show => "show",
        MacControlDockOp::Menu => "menu",
        MacControlDockOp::SelectMenu => "select_menu",
    }
}

fn validate_spaces_request(request: &MacControlSpacesRequest) -> Option<String> {
    match request.op {
        MacControlSpacesOp::List => None,
        MacControlSpacesOp::Switch => {
            let selectors = usize::from(request.space_id.is_some())
                + usize::from(request.space_index.is_some())
                + usize::from(request.direction.is_some());
            if selectors != 1 {
                return Some(
                    "mac_control spaces.switch requires exactly one of spaceId, spaceIndex, or direction."
                        .to_string(),
                );
            }
            None
        }
        MacControlSpacesOp::MoveWindow => {
            if request.space_id.is_none() && request.space_index.is_none() {
                return Some(
                    "mac_control spaces.move_window requires spaceId or spaceIndex.".to_string(),
                );
            }
            if request.window_id.is_none() && request.target.window_title.is_none() {
                return Some(
                    "mac_control spaces.move_window requires windowId or target.windowTitle."
                        .to_string(),
                );
            }
            None
        }
    }
}

fn validate_dialog_request(request: &MacControlDialogRequest) -> Option<String> {
    match request.op {
        MacControlDialogOp::Inspect | MacControlDialogOp::List => None,
        MacControlDialogOp::Click => {
            if request
                .button_text
                .as_deref()
                .is_none_or(|value| value.is_empty())
            {
                return Some("mac_control dialog.click requires buttonText.".to_string());
            }
            None
        }
        MacControlDialogOp::Input => {
            if request.text.as_deref().is_none_or(|value| value.is_empty()) {
                return Some("mac_control dialog.input requires text.".to_string());
            }
            None
        }
        MacControlDialogOp::File => {
            let has_file_work = request
                .file_path
                .as_deref()
                .is_some_and(|value| !value.is_empty())
                || request
                    .file_name
                    .as_deref()
                    .is_some_and(|value| !value.is_empty())
                || request
                    .select_button
                    .as_deref()
                    .is_some_and(|value| !value.is_empty())
                || request
                    .button_text
                    .as_deref()
                    .is_some_and(|value| !value.is_empty());
            if !has_file_work {
                return Some(
                    "mac_control dialog.file requires filePath, fileName, selectButton, or buttonText."
                        .to_string(),
                );
            }
            None
        }
        MacControlDialogOp::Accept | MacControlDialogOp::Dismiss => None,
    }
}

fn apps_op_name(op: MacControlAppsOp) -> &'static str {
    match op {
        MacControlAppsOp::List => "list",
        MacControlAppsOp::Frontmost => "frontmost",
        MacControlAppsOp::Installed => "installed",
        MacControlAppsOp::Search => "search",
        MacControlAppsOp::Activate => "activate",
        MacControlAppsOp::Launch => "launch",
        MacControlAppsOp::Quit => "quit",
    }
}

fn windows_request_has_target(request: &MacControlWindowsRequest) -> bool {
    request
        .window_id
        .as_deref()
        .is_some_and(|value| !value.is_empty())
        || request
            .target
            .window_title
            .as_deref()
            .is_some_and(|value| !value.is_empty())
}

fn validate_windows_request(request: &MacControlWindowsRequest) -> Option<String> {
    match request.op {
        MacControlWindowsOp::Move => {
            if request.x.is_none() || request.y.is_none() {
                return Some("mac_control windows.move requires x and y.".to_string());
            }
        }
        MacControlWindowsOp::Resize => {
            if request.width.is_none() || request.height.is_none() {
                return Some("mac_control windows.resize requires width and height.".to_string());
            }
        }
        MacControlWindowsOp::List
        | MacControlWindowsOp::Focus
        | MacControlWindowsOp::Minimize
        | MacControlWindowsOp::Close => {}
    }
    None
}

fn validate_clipboard_request(request: &MacControlClipboardRequest) -> Option<String> {
    if request.op == MacControlClipboardOp::Set && request.text.is_none() {
        return Some("mac_control clipboard.set requires text.".to_string());
    }
    None
}

fn validate_menu_request(request: &MacControlMenuRequest) -> Option<String> {
    if request.op != MacControlMenuOp::Click {
        return None;
    }
    if request.path.is_empty() && request.menu_index.is_none() {
        return Some("mac_control menu.click requires path or menuIndex.".to_string());
    }
    None
}

fn validate_act_request(request: &MacControlActRequest) -> Option<String> {
    match request.op {
        MacControlActOp::Click => {
            if request.target.is_empty() {
                return Some(
                    "mac_control act.click requires a target; use act.click_point for raw x/y coordinates."
                        .to_string(),
                );
            }
        }
        MacControlActOp::DryRun => {
            if request.target.is_empty() {
                return Some("mac_control act.dry_run requires a target.".to_string());
            }
        }
        MacControlActOp::PerformAction => {
            if request.target.is_empty() {
                return Some("mac_control act.perform_action requires a target.".to_string());
            }
            let Some(ax_action) = request.ax_action.as_deref() else {
                return Some("mac_control act.perform_action requires axAction.".to_string());
            };
            if normalize_perform_ax_action(ax_action).is_none() {
                return Some(
                    "mac_control act.perform_action axAction must be non-empty, at most 128 characters, and contain only ASCII letters, digits, '_' or '-'."
                        .to_string(),
                );
            }
        }
        MacControlActOp::ClickPoint => {
            if request.x.is_none() || request.y.is_none() {
                return Some("mac_control act.click_point requires x and y.".to_string());
            }
            if !request.target.is_empty() {
                return Some("mac_control act.click_point does not accept target; use act.click for AX element targets.".to_string());
            }
        }
        MacControlActOp::MoveCursor => {
            let has_point = request.x.is_some() || request.y.is_some();
            let has_target = !request.target.is_empty();
            if !has_target && (request.x.is_none() || request.y.is_none()) {
                return Some(
                    "mac_control act.move_cursor requires both x and y, or a target.".to_string(),
                );
            }
            if has_target && has_point {
                return Some(
                    "mac_control act.move_cursor accepts either target or x/y, not both."
                        .to_string(),
                );
            }
        }
        MacControlActOp::DoubleClick | MacControlActOp::RightClick => {
            if request.target.is_empty() {
                return Some(format!(
                    "mac_control act.{} requires a target.",
                    act_op_name(request.op)
                ));
            }
        }
        MacControlActOp::Type | MacControlActOp::Paste => {
            if request.text.is_none() {
                return Some(format!(
                    "mac_control act.{} requires text.",
                    act_op_name(request.op)
                ));
            }
        }
        MacControlActOp::SetValue => {
            if request.value.is_none() {
                return Some("mac_control act.set_value requires value.".to_string());
            }
            if request.target.is_empty() {
                return Some("mac_control act.set_value requires a target.".to_string());
            }
        }
        MacControlActOp::Hotkey => {
            if request.key.is_none() && request.keys.is_empty() {
                return Some("mac_control act.hotkey requires key or keys.".to_string());
            }
        }
        MacControlActOp::Press => {
            if request.key.is_none() && request.keys.is_empty() {
                return Some("mac_control act.press requires key or keys.".to_string());
            }
        }
        MacControlActOp::Scroll => {
            if request.delta_x.unwrap_or(0.0) == 0.0 && request.delta_y.unwrap_or(0.0) == 0.0 {
                return Some("mac_control act.scroll requires deltaX or deltaY.".to_string());
            }
        }
        MacControlActOp::Drag => {
            let has_target_source = !request.target.is_empty();
            let has_point_source = request.from_x.is_some() || request.from_y.is_some();
            if !has_target_source && (request.from_x.is_none() || request.from_y.is_none()) {
                return Some(
                    "mac_control act.drag requires a source target or fromX/fromY.".to_string(),
                );
            }
            if has_target_source && has_point_source {
                return Some(
                    "mac_control act.drag accepts either source target or fromX/fromY, not both."
                        .to_string(),
                );
            }
            let has_xy_destination = request.x.is_some() || request.y.is_some();
            let has_to_point = request.to_x.is_some() || request.to_y.is_some();
            let has_to_target = !request.to_target.is_empty();
            let destination_count = usize::from(has_xy_destination)
                + usize::from(has_to_point)
                + usize::from(has_to_target);
            if destination_count == 0 {
                return Some(
                    "mac_control act.drag requires destination x/y, toX/toY, or toTarget."
                        .to_string(),
                );
            }
            if destination_count > 1 {
                return Some(
                    "mac_control act.drag accepts only one destination: x/y, toX/toY, or toTarget."
                        .to_string(),
                );
            }
            if has_xy_destination && (request.x.is_none() || request.y.is_none()) {
                return Some(
                    "mac_control act.drag destination x/y requires both x and y.".to_string(),
                );
            }
            if has_to_point && (request.to_x.is_none() || request.to_y.is_none()) {
                return Some(
                    "mac_control act.drag destination toX/toY requires both toX and toY."
                        .to_string(),
                );
            }
        }
        MacControlActOp::Swipe => {
            let has_point = request.x.is_some() || request.y.is_some();
            let has_from_point = request.from_x.is_some() || request.from_y.is_some();
            let has_target = !request.target.is_empty();
            let source_count =
                usize::from(has_target) + usize::from(has_point) + usize::from(has_from_point);
            if source_count == 0 {
                return Some(
                    "mac_control act.swipe requires start x/y, fromX/fromY, or a target."
                        .to_string(),
                );
            }
            if source_count > 1 {
                return Some(
                    "mac_control act.swipe accepts only one source: target, x/y, or fromX/fromY."
                        .to_string(),
                );
            }
            if has_point && (request.x.is_none() || request.y.is_none()) {
                return Some("mac_control act.swipe start x/y requires both x and y.".to_string());
            }
            if has_from_point && (request.from_x.is_none() || request.from_y.is_none()) {
                return Some(
                    "mac_control act.swipe start fromX/fromY requires both fromX and fromY."
                        .to_string(),
                );
            }
            let has_delta =
                request.delta_x.unwrap_or(0.0) != 0.0 || request.delta_y.unwrap_or(0.0) != 0.0;
            let has_to_point = request.to_x.is_some() || request.to_y.is_some();
            let has_to_target = !request.to_target.is_empty();
            let destination_count =
                usize::from(has_delta) + usize::from(has_to_point) + usize::from(has_to_target);
            if destination_count == 0 {
                return Some(
                    "mac_control act.swipe requires deltaX/deltaY, toX/toY, or toTarget."
                        .to_string(),
                );
            }
            if destination_count > 1 {
                return Some(
                    "mac_control act.swipe accepts only one destination: deltaX/deltaY, toX/toY, or toTarget."
                        .to_string(),
                );
            }
            if has_to_point && (request.to_x.is_none() || request.to_y.is_none()) {
                return Some(
                    "mac_control act.swipe destination toX/toY requires both toX and toY."
                        .to_string(),
                );
            }
        }
    }
    None
}

fn act_op_name(op: MacControlActOp) -> &'static str {
    match op {
        MacControlActOp::Click => "click",
        MacControlActOp::DryRun => "dry_run",
        MacControlActOp::PerformAction => "perform_action",
        MacControlActOp::ClickPoint => "click_point",
        MacControlActOp::MoveCursor => "move_cursor",
        MacControlActOp::DoubleClick => "double_click",
        MacControlActOp::RightClick => "right_click",
        MacControlActOp::Type => "type",
        MacControlActOp::Paste => "paste",
        MacControlActOp::SetValue => "set_value",
        MacControlActOp::Hotkey => "hotkey",
        MacControlActOp::Press => "press",
        MacControlActOp::Scroll => "scroll",
        MacControlActOp::Drag => "drag",
        MacControlActOp::Swipe => "swipe",
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
            stats: runtime_stats(),
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
        stats: runtime_stats(),
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

fn error_stats() -> &'static Mutex<HashMap<String, MacControlErrorStat>> {
    ERROR_STATS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn runtime_stats() -> MacControlRuntimeStats {
    let snapshot_cache_len = snapshot_cache()
        .lock()
        .map(|cache| cache.len())
        .unwrap_or_default();
    let mut recent_errors = error_stats()
        .lock()
        .map(|stats| stats.values().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    recent_errors.sort_by(|a, b| b.last_at.cmp(&a.last_at));
    recent_errors.truncate(MAX_ERROR_STATS);
    MacControlRuntimeStats {
        snapshot_cache_len,
        snapshot_cache_limit: MAX_SNAPSHOT_CACHE,
        screenshot_file_limit: MAX_SCREENSHOT_FILES,
        recent_errors,
    }
}

fn snapshot_cache_summaries(limit: usize) -> Vec<MacControlCachedSnapshotSummary> {
    let limit = limit.max(1).min(HARD_DIAGNOSTICS_LIMIT);
    snapshot_cache()
        .lock()
        .map(|cache| {
            cache
                .iter()
                .rev()
                .take(limit)
                .map(cached_snapshot_summary)
                .collect()
        })
        .unwrap_or_default()
}

fn cached_snapshot_summary(snapshot: &MacControlSnapshot) -> MacControlCachedSnapshotSummary {
    MacControlCachedSnapshotSummary {
        snapshot_id: snapshot.snapshot_id.clone(),
        created_at: snapshot.created_at.clone(),
        frontmost_app: snapshot.frontmost_app.clone(),
        display_count: snapshot.displays.len(),
        window_count: snapshot.windows.len(),
        element_count: snapshot.elements.len(),
        has_screenshot: snapshot.screenshot.is_some(),
        screenshot: snapshot.screenshot.clone(),
        truncated: snapshot.truncated,
        warnings: snapshot.warnings.clone(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MacControlDiagnosticsBundle<'a> {
    status: &'a MacControlStatus,
    result: &'a MacControlDiagnosticsResult,
}

fn create_diagnostics_export_path() -> Result<PathBuf, String> {
    let dir = crate::paths::mac_control_diagnostics_dir()
        .map_err(|e| format!("Unable to resolve macOS control diagnostics directory: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| {
        format!(
            "Unable to create macOS control diagnostics directory {}: {e}",
            dir.display()
        )
    })?;
    Ok(dir.join(format!("macdiag_{}.json", Uuid::new_v4().simple())))
}

fn write_diagnostics_bundle(
    path: &Path,
    status: &MacControlStatus,
    result: &MacControlDiagnosticsResult,
) -> Result<(), String> {
    let bundle = MacControlDiagnosticsBundle { status, result };
    let bytes = serde_json::to_vec_pretty(&bundle)
        .map_err(|e| format!("Unable to serialize macOS control diagnostics: {e}"))?;
    fs::write(path, bytes).map_err(|e| {
        format!(
            "Unable to write macOS control diagnostics bundle {}: {e}",
            path.display()
        )
    })
}

fn record_error(operation: &str, message: &str) {
    let Ok(mut stats) = error_stats().lock() else {
        return;
    };
    let key = format!("{operation}:{message}");
    let now = Utc::now().to_rfc3339();
    let entry = stats.entry(key).or_insert_with(|| MacControlErrorStat {
        operation: operation.to_string(),
        message: message.to_string(),
        count: 0,
        last_at: now.clone(),
    });
    entry.count = entry.count.saturating_add(1);
    entry.last_at = now;
    if stats.len() > MAX_ERROR_STATS * 2 {
        let mut entries = stats
            .iter()
            .map(|(key, value)| (key.clone(), value.last_at.clone()))
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| a.1.cmp(&b.1));
        for (key, _) in entries
            .into_iter()
            .take(stats.len().saturating_sub(MAX_ERROR_STATS))
        {
            stats.remove(&key);
        }
    }
}

pub fn record_snapshot(snapshot: MacControlSnapshot) {
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

fn visual_observe_result(
    snapshot: MacControlSnapshot,
    annotate: bool,
    ui_map_limit: usize,
) -> MacControlVisualResult {
    let screenshot = snapshot.screenshot.clone();
    let mut warnings = snapshot.warnings.clone();
    let (annotated_screenshot, ui_map) = if annotate {
        match build_visual_annotations(&snapshot, ui_map_limit) {
            Ok(annotations) => annotations,
            Err(error) => {
                warnings.push(error);
                (None, Vec::new())
            }
        }
    } else {
        (None, Vec::new())
    };
    MacControlVisualResult {
        op: MacControlVisualOp::Observe,
        snapshot_id: Some(snapshot.snapshot_id.clone()),
        snapshot: Some(snapshot),
        screenshot,
        annotated_screenshot,
        ui_map,
        coordinate_space: None,
        image_point: None,
        screen_point: None,
        inside_frame: None,
        hit_elements: Vec::new(),
        nearest_elements: Vec::new(),
        text_blocks: Vec::new(),
        text_matches: Vec::new(),
        suggested_action: None,
        warnings,
    }
}

async fn visual_snapshot_for_ocr(
    bridge: &dyn MacControlBridge,
    request: &MacControlVisualRequest,
) -> Result<MacControlSnapshot, String> {
    if let Some(snapshot_id) = request.snapshot_id.as_deref() {
        return cached_snapshot(snapshot_id).ok_or_else(|| {
            format!(
                "mac_control visual OCR snapshotId '{snapshot_id}' was not found or expired; call visual.observe again."
            )
        });
    }

    let snapshot = bridge
        .snapshot(MacControlSnapshotRequest {
            include_screenshot: true,
            screenshot_target: request.screenshot_target,
            display_id: request.display_id,
            window_id: request.window_id.clone(),
            max_elements: request.max_elements,
            max_depth: request.max_depth,
        })
        .await?;
    record_snapshot(snapshot.clone());
    Ok(snapshot)
}

fn build_visual_annotations(
    snapshot: &MacControlSnapshot,
    limit: usize,
) -> Result<
    (
        Option<MacControlScreenshotSummary>,
        Vec<MacControlUiMapItem>,
    ),
    String,
> {
    let ui_map = build_ui_map(snapshot, limit)?;
    let screenshot = render_annotated_screenshot(snapshot, &ui_map)?;
    Ok((Some(screenshot), ui_map))
}

fn build_ui_map(
    snapshot: &MacControlSnapshot,
    limit: usize,
) -> Result<Vec<MacControlUiMapItem>, String> {
    let (screenshot, frame, scale) =
        visual_screenshot_metadata(snapshot, "visual.observe annotate")?;
    let limit = limit.max(1).min(HARD_UI_MAP_LIMIT);
    let mut items = snapshot
        .elements
        .iter()
        .filter_map(|element| {
            let bounds = element.bounds_points?;
            let image_bounds =
                screen_bounds_to_image_bounds(bounds, frame, scale).and_then(|bounds| {
                    clamp_image_bounds(
                        bounds,
                        screenshot.width_px as f64,
                        screenshot.height_px as f64,
                    )
                })?;
            if image_bounds.width < 4.0 || image_bounds.height < 4.0 {
                return None;
            }
            if !should_include_ui_map_element(element, image_bounds, screenshot) {
                return None;
            }
            let item = MacControlUiMapItem {
                id: element.id.clone(),
                window_id: element.window_id.clone(),
                role: element.role.clone(),
                text: ui_map_text(element),
                enabled: element.enabled,
                focused: element.focused,
                bounds_points: bounds,
                image_bounds,
                actions: element.actions.clone(),
            };
            Some((
                ui_map_priority(element, image_bounds),
                image_bounds.y,
                image_bounds.x,
                item,
            ))
        })
        .collect::<Vec<_>>();

    items.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| a.3.id.cmp(&b.3.id))
    });
    items.truncate(limit);
    items.sort_by(|a, b| {
        a.1.partial_cmp(&b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| a.3.id.cmp(&b.3.id))
    });

    Ok(items.into_iter().map(|(_, _, _, item)| item).collect())
}

fn should_include_ui_map_element(
    element: &MacControlElementSummary,
    image_bounds: MacControlBounds,
    screenshot: &MacControlScreenshotSummary,
) -> bool {
    let role = element.role.as_deref().unwrap_or_default();
    let role_lc = role.to_ascii_lowercase();
    if role_lc == "axwindow" {
        return false;
    }
    let area = bounds_area(image_bounds);
    let screenshot_area = f64::from(screenshot.width_px) * f64::from(screenshot.height_px);
    if screenshot_area > 0.0
        && area > screenshot_area * 0.75
        && !element.focused
        && !actionable_element(element)
    {
        return false;
    }
    actionable_element(element)
        || element.focused
        || matches!(
            role,
            "AXButton"
                | "AXCheckBox"
                | "AXRadioButton"
                | "AXPopUpButton"
                | "AXMenuButton"
                | "AXComboBox"
                | "AXTextField"
                | "AXTextArea"
                | "AXSearchField"
                | "AXSlider"
                | "AXTab"
                | "AXLink"
        )
}

fn ui_map_priority(element: &MacControlElementSummary, image_bounds: MacControlBounds) -> i32 {
    let mut priority = 0;
    if actionable_element(element) {
        priority += 100;
    }
    if element.focused {
        priority += 40;
    }
    if element.enabled == Some(false) {
        priority -= 30;
    }
    if ui_map_text(element).is_some() {
        priority += 10;
    }
    if bounds_area(image_bounds) <= 120_000.0 {
        priority += 5;
    }
    priority
}

fn ui_map_text(element: &MacControlElementSummary) -> Option<String> {
    element
        .label
        .as_deref()
        .or(element.value.as_deref())
        .and_then(|text| normalize_optional_string(Some(text.to_string())))
        .map(|text| truncate_chars(&text, 80))
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let keep = max_chars.saturating_sub(3);
    format!("{}...", text.chars().take(keep).collect::<String>())
}

fn render_annotated_screenshot(
    snapshot: &MacControlSnapshot,
    ui_map: &[MacControlUiMapItem],
) -> Result<MacControlScreenshotSummary, String> {
    let screenshot = snapshot.screenshot.as_ref().ok_or_else(|| {
        format!(
            "mac_control visual.observe annotate snapshotId '{}' has no screenshot metadata.",
            snapshot.snapshot_id
        )
    })?;
    let mut image = image::open(&screenshot.path)
        .map_err(|e| {
            format!(
                "Unable to open macOS control screenshot {} for annotation: {e}",
                screenshot.path
            )
        })?
        .to_rgba8();
    let palette = annotation_palette();
    for (idx, item) in ui_map.iter().enumerate() {
        let color = palette[idx % palette.len()];
        draw_rect_outline(&mut image, item.image_bounds, color, 2);
        draw_label(&mut image, item.image_bounds, &item.id, color);
    }

    let mut bytes = Vec::new();
    let rgb = image::DynamicImage::ImageRgba8(image).to_rgb8();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 86);
    encoder
        .encode_image(&rgb)
        .map_err(|e| format!("Unable to encode annotated macOS control screenshot: {e}"))?;
    store_annotated_screenshot_jpeg(screenshot, &bytes)
}

fn store_annotated_screenshot_jpeg(
    original: &MacControlScreenshotSummary,
    bytes: &[u8],
) -> Result<MacControlScreenshotSummary, String> {
    let media_id = sanitize_media_id(&format!("{}_annotated", original.media_id))?;
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
            "Unable to write annotated macOS control screenshot {}: {e}",
            path.display()
        )
    })?;
    prune_screenshot_files(&dir);

    let mut summary = original.clone();
    summary.media_id = media_id;
    summary.path = path.display().to_string();
    Ok(summary)
}

fn screen_bounds_to_image_bounds(
    screen_bounds: MacControlBounds,
    frame: MacControlBounds,
    scale: f64,
) -> Option<MacControlBounds> {
    if !screen_bounds.x.is_finite()
        || !screen_bounds.y.is_finite()
        || !screen_bounds.width.is_finite()
        || !screen_bounds.height.is_finite()
        || !scale.is_finite()
        || scale <= 0.0
    {
        return None;
    }
    Some(MacControlBounds {
        x: (screen_bounds.x - frame.x) * scale,
        y: (screen_bounds.y - frame.y) * scale,
        width: screen_bounds.width * scale,
        height: screen_bounds.height * scale,
    })
}

fn draw_rect_outline(
    image: &mut image::RgbaImage,
    bounds: MacControlBounds,
    color: image::Rgba<u8>,
    thickness: u32,
) {
    let Some((x0, y0, x1, y1)) = image_bounds_to_pixel_rect(bounds, image.width(), image.height())
    else {
        return;
    };
    let thickness = thickness.max(1);
    for offset in 0..thickness {
        let top = y0.saturating_add(offset).min(y1);
        let bottom = y1.saturating_sub(offset).max(y0);
        for x in x0..=x1 {
            put_pixel_checked(image, x, top, color);
            put_pixel_checked(image, x, bottom, color);
        }
        let left = x0.saturating_add(offset).min(x1);
        let right = x1.saturating_sub(offset).max(x0);
        for y in y0..=y1 {
            put_pixel_checked(image, left, y, color);
            put_pixel_checked(image, right, y, color);
        }
    }
}

fn draw_label(
    image: &mut image::RgbaImage,
    bounds: MacControlBounds,
    label: &str,
    color: image::Rgba<u8>,
) {
    let Some((x0, y0, _, _)) = image_bounds_to_pixel_rect(bounds, image.width(), image.height())
    else {
        return;
    };
    let scale = 2;
    let padding = 3;
    let text_width = bitmap_text_width(label, scale);
    let box_width = (text_width + padding * 2).min(image.width().max(1));
    let box_height = 7 * scale + padding * 2;
    let label_x = x0.min(image.width().saturating_sub(box_width));
    let label_y = if y0 > box_height + 1 {
        y0 - box_height - 1
    } else {
        y0
    }
    .min(image.height().saturating_sub(box_height));
    fill_rect(image, label_x, label_y, box_width, box_height, color);
    draw_bitmap_text(
        image,
        label_x + padding,
        label_y + padding,
        label,
        scale,
        image::Rgba([255, 255, 255, 255]),
    );
}

fn image_bounds_to_pixel_rect(
    bounds: MacControlBounds,
    width: u32,
    height: u32,
) -> Option<(u32, u32, u32, u32)> {
    if width == 0 || height == 0 {
        return None;
    }
    let clamped = clamp_image_bounds(bounds, f64::from(width), f64::from(height))?;
    let x0 = clamped.x.floor().max(0.0).min(f64::from(width - 1)) as u32;
    let y0 = clamped.y.floor().max(0.0).min(f64::from(height - 1)) as u32;
    let x1 = (clamped.x + clamped.width)
        .ceil()
        .max(1.0)
        .min(f64::from(width)) as u32;
    let y1 = (clamped.y + clamped.height)
        .ceil()
        .max(1.0)
        .min(f64::from(height)) as u32;
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - 1, y1 - 1))
}

fn fill_rect(
    image: &mut image::RgbaImage,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    color: image::Rgba<u8>,
) {
    for yy in y..y.saturating_add(height).min(image.height()) {
        for xx in x..x.saturating_add(width).min(image.width()) {
            put_pixel_checked(image, xx, yy, color);
        }
    }
}

fn draw_bitmap_text(
    image: &mut image::RgbaImage,
    x: u32,
    y: u32,
    text: &str,
    scale: u32,
    color: image::Rgba<u8>,
) {
    let mut cursor = x;
    for ch in text.chars() {
        draw_bitmap_char(image, cursor, y, ch, scale, color);
        cursor = cursor.saturating_add((5 * scale) + scale);
    }
}

fn draw_bitmap_char(
    image: &mut image::RgbaImage,
    x: u32,
    y: u32,
    ch: char,
    scale: u32,
    color: image::Rgba<u8>,
) {
    for (row_idx, row) in bitmap_glyph(ch).iter().enumerate() {
        for (col_idx, bit) in row.as_bytes().iter().enumerate() {
            if *bit != b'1' {
                continue;
            }
            let px = x + (col_idx as u32 * scale);
            let py = y + (row_idx as u32 * scale);
            for yy in py..py.saturating_add(scale) {
                for xx in px..px.saturating_add(scale) {
                    put_pixel_checked(image, xx, yy, color);
                }
            }
        }
    }
}

fn bitmap_text_width(text: &str, scale: u32) -> u32 {
    let chars = text.chars().count() as u32;
    if chars == 0 {
        0
    } else {
        chars * 5 * scale + chars.saturating_sub(1) * scale
    }
}

fn put_pixel_checked(image: &mut image::RgbaImage, x: u32, y: u32, color: image::Rgba<u8>) {
    if x < image.width() && y < image.height() {
        image.put_pixel(x, y, color);
    }
}

fn annotation_palette() -> [image::Rgba<u8>; 8] {
    [
        image::Rgba([0, 122, 255, 255]),
        image::Rgba([255, 45, 85, 255]),
        image::Rgba([52, 199, 89, 255]),
        image::Rgba([255, 149, 0, 255]),
        image::Rgba([175, 82, 222, 255]),
        image::Rgba([90, 200, 250, 255]),
        image::Rgba([255, 204, 0, 255]),
        image::Rgba([255, 59, 48, 255]),
    ]
}

fn bitmap_glyph(ch: char) -> [&'static str; 7] {
    match ch.to_ascii_lowercase() {
        '0' => [
            "01110", "10001", "10011", "10101", "11001", "10001", "01110",
        ],
        '1' => [
            "00100", "01100", "00100", "00100", "00100", "00100", "01110",
        ],
        '2' => [
            "01110", "10001", "00001", "00010", "00100", "01000", "11111",
        ],
        '3' => [
            "11110", "00001", "00001", "01110", "00001", "00001", "11110",
        ],
        '4' => [
            "00010", "00110", "01010", "10010", "11111", "00010", "00010",
        ],
        '5' => [
            "11111", "10000", "10000", "11110", "00001", "00001", "11110",
        ],
        '6' => [
            "01110", "10000", "10000", "11110", "10001", "10001", "01110",
        ],
        '7' => [
            "11111", "00001", "00010", "00100", "01000", "01000", "01000",
        ],
        '8' => [
            "01110", "10001", "10001", "01110", "10001", "10001", "01110",
        ],
        '9' => [
            "01110", "10001", "10001", "01111", "00001", "00001", "01110",
        ],
        'e' => [
            "00000", "01110", "10001", "11111", "10000", "10001", "01110",
        ],
        'l' => [
            "11000", "01000", "01000", "01000", "01000", "01000", "11100",
        ],
        '_' => [
            "00000", "00000", "00000", "00000", "00000", "00000", "11111",
        ],
        '-' => [
            "00000", "00000", "00000", "11111", "00000", "00000", "00000",
        ],
        _ => [
            "11111", "10001", "00010", "00100", "00100", "00000", "00100",
        ],
    }
}

fn resolve_visual_point(
    snapshot: &MacControlSnapshot,
    coordinate_space: MacControlCoordinateSpace,
    x: f64,
    y: f64,
    limit: usize,
) -> Result<MacControlVisualResult, String> {
    let screenshot = snapshot.screenshot.as_ref().ok_or_else(|| {
        format!(
            "mac_control visual.point snapshotId '{}' has no screenshot metadata; call visual.observe or snapshot includeScreenshot=true.",
            snapshot.snapshot_id
        )
    })?;
    let frame = screenshot.bounds_points.ok_or_else(|| {
        format!(
            "mac_control visual.point snapshotId '{}' has no screenshot bounds metadata.",
            snapshot.snapshot_id
        )
    })?;
    let scale = screenshot.scale.ok_or_else(|| {
        format!(
            "mac_control visual.point snapshotId '{}' has no screenshot scale metadata.",
            snapshot.snapshot_id
        )
    })?;
    if !scale.is_finite() || scale <= 0.0 {
        return Err(format!(
            "mac_control visual.point snapshotId '{}' has invalid screenshot scale.",
            snapshot.snapshot_id
        ));
    }
    if frame.width <= 0.0 || frame.height <= 0.0 {
        return Err(format!(
            "mac_control visual.point snapshotId '{}' has invalid screenshot bounds.",
            snapshot.snapshot_id
        ));
    }

    let (image_point, screen_point) = match coordinate_space {
        MacControlCoordinateSpace::ImagePixels => (
            MacControlPoint { x, y },
            MacControlPoint {
                x: frame.x + x / scale,
                y: frame.y + y / scale,
            },
        ),
        MacControlCoordinateSpace::ScreenPoints => (
            MacControlPoint {
                x: (x - frame.x) * scale,
                y: (y - frame.y) * scale,
            },
            MacControlPoint { x, y },
        ),
    };

    let inside_frame = image_point.x >= 0.0
        && image_point.y >= 0.0
        && image_point.x < screenshot.width_px as f64
        && image_point.y < screenshot.height_px as f64;
    let (hit_elements, nearest_elements) =
        visual_element_matches(snapshot, screen_point, limit.max(1), inside_frame);
    let suggested_action = inside_frame.then_some(MacControlSuggestedAction {
        action: "act".to_string(),
        op: MacControlActOp::ClickPoint,
        x: screen_point.x,
        y: screen_point.y,
    });

    Ok(MacControlVisualResult {
        op: MacControlVisualOp::Point,
        snapshot_id: Some(snapshot.snapshot_id.clone()),
        snapshot: None,
        screenshot: Some(screenshot.clone()),
        annotated_screenshot: None,
        ui_map: Vec::new(),
        coordinate_space: Some(coordinate_space),
        image_point: Some(image_point),
        screen_point: Some(screen_point),
        inside_frame: Some(inside_frame),
        hit_elements,
        nearest_elements,
        text_blocks: Vec::new(),
        text_matches: Vec::new(),
        suggested_action,
        warnings: if inside_frame {
            Vec::new()
        } else {
            vec!["The resolved point is outside the captured screenshot frame.".to_string()]
        },
    })
}

fn resolve_ocr_text_blocks(
    snapshot: &MacControlSnapshot,
    raw_blocks: Vec<MacControlOcrRawTextBlock>,
    min_confidence: Option<f32>,
) -> Result<Vec<MacControlOcrTextBlock>, String> {
    let (screenshot, frame, scale) = visual_screenshot_metadata(snapshot, "visual OCR")?;
    let min_confidence = min_confidence.unwrap_or(0.0);
    let mut blocks = raw_blocks
        .into_iter()
        .filter_map(|block| {
            let text = block.text.trim().to_string();
            if text.is_empty() || !block.confidence.is_finite() || block.confidence < min_confidence
            {
                return None;
            }
            let image_bounds = clamp_image_bounds(
                block.image_bounds,
                screenshot.width_px as f64,
                screenshot.height_px as f64,
            )?;
            let screen_bounds = image_bounds_to_screen_bounds(image_bounds, frame, scale);
            Some((text, block.confidence, image_bounds, screen_bounds))
        })
        .collect::<Vec<_>>();

    blocks.sort_by(|a, b| {
        a.2.y
            .partial_cmp(&b.2.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                a.2.x
                    .partial_cmp(&b.2.x)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.0.cmp(&b.0))
    });

    Ok(blocks
        .into_iter()
        .enumerate()
        .map(
            |(idx, (text, confidence, image_bounds, screen_bounds))| MacControlOcrTextBlock {
                id: format!("txt_{}", idx + 1),
                text,
                confidence,
                image_bounds,
                screen_bounds,
                image_point: bounds_center(image_bounds),
                screen_point: bounds_center(screen_bounds),
            },
        )
        .collect())
}

fn resolve_ocr_text_matches(
    snapshot: &MacControlSnapshot,
    text_blocks: &[MacControlOcrTextBlock],
    query: &str,
    strategy: MacControlStringMatch,
    limit: usize,
) -> Vec<MacControlOcrTextMatch> {
    let limit = limit.max(1);
    let mut matches = text_blocks
        .iter()
        .filter_map(|block| {
            if !string_matches(Some(&block.text), query, strategy) {
                return None;
            }
            let exact = block.text.eq_ignore_ascii_case(query);
            let score = if exact { 100 } else { 80 };
            let mut reasons = vec![match strategy {
                MacControlStringMatch::Exact => "text_exact".to_string(),
                MacControlStringMatch::Contains => "text_contains".to_string(),
            }];
            if block.confidence >= 0.8 {
                reasons.push("high_confidence".to_string());
            }
            Some((block.clone(), score, reasons))
        })
        .collect::<Vec<_>>();

    matches.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| {
                b.0.confidence
                    .partial_cmp(&a.0.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                bounds_area(a.0.screen_bounds)
                    .partial_cmp(&bounds_area(b.0.screen_bounds))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.0.id.cmp(&b.0.id))
    });

    matches
        .into_iter()
        .take(limit)
        .map(|(block, score, reasons)| {
            let (hit_elements, nearest_elements) =
                visual_element_matches(snapshot, block.screen_point, limit, true);
            let suggested_action = Some(MacControlSuggestedAction {
                action: "act".to_string(),
                op: MacControlActOp::ClickPoint,
                x: block.screen_point.x,
                y: block.screen_point.y,
            });
            MacControlOcrTextMatch {
                block,
                score,
                reasons,
                hit_elements,
                nearest_elements,
                suggested_action,
            }
        })
        .collect()
}

fn visual_screenshot_metadata<'a>(
    snapshot: &'a MacControlSnapshot,
    operation: &str,
) -> Result<(&'a MacControlScreenshotSummary, MacControlBounds, f64), String> {
    let screenshot = snapshot.screenshot.as_ref().ok_or_else(|| {
        format!(
            "mac_control {operation} snapshotId '{}' has no screenshot metadata; call visual.observe or snapshot includeScreenshot=true.",
            snapshot.snapshot_id
        )
    })?;
    let frame = screenshot.bounds_points.ok_or_else(|| {
        format!(
            "mac_control {operation} snapshotId '{}' has no screenshot bounds metadata.",
            snapshot.snapshot_id
        )
    })?;
    let scale = screenshot.scale.ok_or_else(|| {
        format!(
            "mac_control {operation} snapshotId '{}' has no screenshot scale metadata.",
            snapshot.snapshot_id
        )
    })?;
    if !scale.is_finite() || scale <= 0.0 {
        return Err(format!(
            "mac_control {operation} snapshotId '{}' has invalid screenshot scale.",
            snapshot.snapshot_id
        ));
    }
    if frame.width <= 0.0 || frame.height <= 0.0 {
        return Err(format!(
            "mac_control {operation} snapshotId '{}' has invalid screenshot bounds.",
            snapshot.snapshot_id
        ));
    }
    Ok((screenshot, frame, scale))
}

fn clamp_image_bounds(
    bounds: MacControlBounds,
    width_px: f64,
    height_px: f64,
) -> Option<MacControlBounds> {
    if !bounds.x.is_finite()
        || !bounds.y.is_finite()
        || !bounds.width.is_finite()
        || !bounds.height.is_finite()
        || width_px <= 0.0
        || height_px <= 0.0
    {
        return None;
    }
    let x1 = bounds.x.max(0.0).min(width_px);
    let y1 = bounds.y.max(0.0).min(height_px);
    let x2 = (bounds.x + bounds.width).max(0.0).min(width_px);
    let y2 = (bounds.y + bounds.height).max(0.0).min(height_px);
    (x2 > x1 && y2 > y1).then_some(MacControlBounds {
        x: x1,
        y: y1,
        width: x2 - x1,
        height: y2 - y1,
    })
}

fn image_bounds_to_screen_bounds(
    image_bounds: MacControlBounds,
    frame: MacControlBounds,
    scale: f64,
) -> MacControlBounds {
    MacControlBounds {
        x: frame.x + image_bounds.x / scale,
        y: frame.y + image_bounds.y / scale,
        width: image_bounds.width / scale,
        height: image_bounds.height / scale,
    }
}

fn bounds_center(bounds: MacControlBounds) -> MacControlPoint {
    MacControlPoint {
        x: bounds.x + bounds.width / 2.0,
        y: bounds.y + bounds.height / 2.0,
    }
}

fn bounds_area(bounds: MacControlBounds) -> f64 {
    bounds.width.max(0.0) * bounds.height.max(0.0)
}

fn visual_element_matches(
    snapshot: &MacControlSnapshot,
    point: MacControlPoint,
    limit: usize,
    allow_hits: bool,
) -> (
    Vec<MacControlVisualElementMatch>,
    Vec<MacControlVisualElementMatch>,
) {
    let mut matches = snapshot
        .elements
        .iter()
        .filter_map(|element| {
            let bounds = element.bounds_points?;
            let distance = distance_to_bounds(point, bounds);
            Some(MacControlVisualElementMatch {
                element: element.clone(),
                window: element
                    .window_id
                    .as_deref()
                    .and_then(|id| snapshot.windows.iter().find(|window| window.id == id))
                    .cloned(),
                contains_point: distance == 0.0,
                distance_points: distance,
            })
        })
        .collect::<Vec<_>>();

    matches.sort_by(visual_match_order);
    let hit_elements = if allow_hits {
        matches
            .iter()
            .filter(|item| item.contains_point)
            .take(limit)
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let nearest_elements = matches
        .into_iter()
        .filter(|item| allow_hits || !item.contains_point)
        .take(limit)
        .collect::<Vec<_>>();
    (hit_elements, nearest_elements)
}

fn visual_match_order(
    a: &MacControlVisualElementMatch,
    b: &MacControlVisualElementMatch,
) -> std::cmp::Ordering {
    b.contains_point
        .cmp(&a.contains_point)
        .then_with(|| {
            a.distance_points
                .partial_cmp(&b.distance_points)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| {
            element_area(&a.element)
                .partial_cmp(&element_area(&b.element))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| actionable_element(&b.element).cmp(&actionable_element(&a.element)))
        .then_with(|| a.element.id.cmp(&b.element.id))
}

fn actionable_element(element: &MacControlElementSummary) -> bool {
    element.enabled != Some(false)
        && element.actions.iter().any(|action| {
            matches!(
                action.as_str(),
                "AXPress" | "AXConfirm" | "AXShowMenu" | "AXRaise" | "AXPick" | "AXOpen"
            )
        })
}

fn element_area(element: &MacControlElementSummary) -> f64 {
    element
        .bounds_points
        .map(|bounds| (bounds.width.max(0.0)) * (bounds.height.max(0.0)))
        .unwrap_or(f64::INFINITY)
}

fn distance_to_bounds(point: MacControlPoint, bounds: MacControlBounds) -> f64 {
    let max_x = bounds.x + bounds.width;
    let max_y = bounds.y + bounds.height;
    let dx = if point.x < bounds.x {
        bounds.x - point.x
    } else if point.x > max_x {
        point.x - max_x
    } else {
        0.0
    };
    let dy = if point.y < bounds.y {
        bounds.y - point.y
    } else if point.y > max_y {
        point.y - max_y
    } else {
        0.0
    };
    dx.hypot(dy)
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

fn wait_condition_satisfied(op: MacControlWaitOp, target_matched: bool) -> bool {
    match op {
        MacControlWaitOp::Present => target_matched,
        MacControlWaitOp::Gone => !target_matched,
    }
}

fn timeout_wait_message(op: MacControlWaitOp, timeout_ms: u64) -> String {
    match op {
        MacControlWaitOp::Present => {
            format!("Timed out waiting for macOS target after {timeout_ms} ms.")
        }
        MacControlWaitOp::Gone => {
            format!("Timed out waiting for macOS target to disappear after {timeout_ms} ms.")
        }
    }
}

fn app_matches_target(app: &MacControlAppSummary, target: &MacControlTargetQuery) -> bool {
    optional_contains(app.name.as_deref(), target.app_name.as_deref())
        && optional_contains(app.bundle_id.as_deref(), target.bundle_id.as_deref())
}

fn window_matches_target(window: &MacControlWindowSummary, target: &MacControlTargetQuery) -> bool {
    optional_string_match(
        window.title.as_deref(),
        target.window_title.as_deref(),
        target.window_title_match,
    )
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
                .is_some_and(|window| {
                    string_matches(window.title.as_deref(), query, target.window_title_match)
                })
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

fn optional_string_match(
    actual: Option<&str>,
    query: Option<&str>,
    strategy: MacControlStringMatch,
) -> bool {
    query
        .filter(|query| !query.is_empty())
        .map_or(true, |query| string_matches(actual, query, strategy))
}

fn string_matches(actual: Option<&str>, query: &str, strategy: MacControlStringMatch) -> bool {
    actual
        .map(|actual| match strategy {
            MacControlStringMatch::Exact => actual.eq_ignore_ascii_case(query),
            MacControlStringMatch::Contains => contains_ci(Some(actual), query),
        })
        .unwrap_or(false)
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
                role: Some("AXWindow".to_string()),
                subrole: Some("AXStandardWindow".to_string()),
                title: Some("Downloads".to_string()),
                focused: true,
                bounds_points: Some(MacControlBounds {
                    x: 50.0,
                    y: 80.0,
                    width: 400.0,
                    height: 300.0,
                }),
            }],
            elements: vec![
                MacControlElementSummary {
                    id: "el_window".to_string(),
                    window_id: Some("win_1".to_string()),
                    role: Some("AXWindow".to_string()),
                    label: Some("Downloads".to_string()),
                    value: None,
                    enabled: Some(true),
                    focused: false,
                    bounds_points: Some(MacControlBounds {
                        x: 50.0,
                        y: 80.0,
                        width: 400.0,
                        height: 300.0,
                    }),
                    actions: vec!["AXRaise".to_string()],
                },
                MacControlElementSummary {
                    id: "el_1".to_string(),
                    window_id: Some("win_1".to_string()),
                    role: Some("AXButton".to_string()),
                    label: Some("Open".to_string()),
                    value: None,
                    enabled: Some(true),
                    focused: false,
                    bounds_points: Some(MacControlBounds {
                        x: 120.0,
                        y: 130.0,
                        width: 80.0,
                        height: 30.0,
                    }),
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
                    bounds_points: Some(MacControlBounds {
                        x: 100.0,
                        y: 100.0,
                        width: 300.0,
                        height: 80.0,
                    }),
                    actions: Vec::new(),
                },
            ],
            screenshot: Some(MacControlScreenshotSummary {
                media_id: "macsnap_sample".to_string(),
                path: "/tmp/macsnap_sample.jpg".to_string(),
                width_px: 800,
                height_px: 600,
                target: MacControlScreenshotTarget::Window,
                display_id: Some(1),
                window_id: Some("win_1".to_string()),
                window_title: Some("Downloads".to_string()),
                bounds_points: Some(MacControlBounds {
                    x: 50.0,
                    y: 80.0,
                    width: 400.0,
                    height: 300.0,
                }),
                scale: Some(2.0),
            }),
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
            ..Default::default()
        }
        .clamped();

        assert!(request.include_screenshot);
        assert_eq!(request.max_elements, HARD_SNAPSHOT_MAX_ELEMENTS);
        assert_eq!(request.max_depth, HARD_SNAPSHOT_MAX_DEPTH);
    }

    #[test]
    fn snapshot_request_normalizes_screenshot_target_fields() {
        let request = MacControlSnapshotRequest {
            include_screenshot: true,
            screenshot_target: MacControlScreenshotTarget::Window,
            window_id: Some(" win_2 ".to_string()),
            display_id: Some(42),
            ..Default::default()
        }
        .clamped();

        assert!(request.include_screenshot);
        assert_eq!(
            request.screenshot_target,
            MacControlScreenshotTarget::Window
        );
        assert_eq!(request.window_id.as_deref(), Some("win_2"));
        assert_eq!(request.display_id, Some(42));
    }

    #[test]
    fn snapshot_request_accepts_screenshot_target_aliases() {
        let screen: MacControlSnapshotRequest = serde_json::from_value(serde_json::json!({
            "includeScreenshot": true,
            "screenshotTarget": "screen"
        }))
        .unwrap();
        let window: MacControlSnapshotRequest = serde_json::from_value(serde_json::json!({
            "includeScreenshot": true,
            "screenshotTarget": "frontmost_window"
        }))
        .unwrap();

        assert_eq!(
            screen.screenshot_target,
            MacControlScreenshotTarget::Display
        );
        assert_eq!(window.screenshot_target, MacControlScreenshotTarget::Window);
    }

    #[test]
    fn visual_request_clamps_and_defaults() {
        let request = MacControlVisualRequest {
            max_elements: 10_000,
            max_depth: 100,
            limit: 0,
            ui_map_limit: 999,
            languages: vec![
                " zh-Hans ".to_string(),
                " ".to_string(),
                "en-US".to_string(),
            ],
            min_confidence: Some(2.0),
            ..Default::default()
        }
        .clamped();

        assert_eq!(request.op, MacControlVisualOp::Observe);
        assert_eq!(
            request.coordinate_space,
            MacControlCoordinateSpace::ImagePixels
        );
        assert_eq!(request.max_elements, HARD_SNAPSHOT_MAX_ELEMENTS);
        assert_eq!(request.max_depth, HARD_SNAPSHOT_MAX_DEPTH);
        assert_eq!(request.limit, DEFAULT_VISUAL_LIMIT);
        assert_eq!(request.ui_map_limit, HARD_UI_MAP_LIMIT);
        assert_eq!(request.languages, vec!["zh-Hans", "en-US"]);
        assert_eq!(request.min_confidence, Some(1.0));

        let point: MacControlVisualRequest = serde_json::from_value(serde_json::json!({
            "op": "point",
            "coordinateSpace": "screen_points",
            "snapshotId": " macsnap_1 ",
            "x": 0,
            "y": 0,
            "limit": 999
        }))
        .expect("visual point request");
        let point = point.clamped();

        assert_eq!(point.op, MacControlVisualOp::Point);
        assert_eq!(
            point.coordinate_space,
            MacControlCoordinateSpace::ScreenPoints
        );
        assert_eq!(point.snapshot_id.as_deref(), Some("macsnap_1"));
        assert_eq!(point.limit, HARD_ELEMENTS_LIMIT);
        assert_eq!(point.x, Some(0.0));
        assert_eq!(point.y, Some(0.0));

        let find_text: MacControlVisualRequest = serde_json::from_value(serde_json::json!({
            "op": "find_text",
            "text": " Save ",
            "textMatch": "contains",
            "recognitionLevel": "fast",
            "minConfidence": 0.42
        }))
        .expect("visual find_text request");
        let find_text = find_text.clamped();

        assert_eq!(find_text.op, MacControlVisualOp::FindText);
        assert_eq!(find_text.text.as_deref(), Some("Save"));
        assert_eq!(find_text.text_match, MacControlStringMatch::Contains);
        assert_eq!(
            find_text.recognition_level,
            MacControlOcrRecognitionLevel::Fast
        );
        assert_eq!(find_text.min_confidence, Some(0.42));
    }

    #[test]
    fn visual_ui_map_maps_bounds_to_image_pixels() {
        let snapshot = sample_snapshot();
        let ui_map = build_ui_map(&snapshot, 10).expect("ui map");

        assert!(ui_map.iter().all(|item| item.id != "el_window"));
        let button = ui_map
            .iter()
            .find(|item| item.id == "el_1")
            .expect("button item");
        assert_eq!(button.text.as_deref(), Some("Open"));
        assert_eq!(button.image_bounds.x, 140.0);
        assert_eq!(button.image_bounds.y, 100.0);
        assert_eq!(button.image_bounds.width, 160.0);
        assert_eq!(button.image_bounds.height, 60.0);

        let text = ui_map
            .iter()
            .find(|item| item.id == "el_2")
            .expect("text item");
        assert_eq!(text.text.as_deref(), Some("Search Downloads"));
        assert!(text.focused);
    }

    #[test]
    fn visual_point_maps_image_pixels_and_hits_smallest_actionable_element() {
        let snapshot = sample_snapshot();
        let result = resolve_visual_point(
            &snapshot,
            MacControlCoordinateSpace::ImagePixels,
            200.0,
            130.0,
            5,
        )
        .expect("visual point");

        assert_eq!(result.snapshot_id.as_deref(), Some("macsnap_sample"));
        assert_eq!(
            result.coordinate_space,
            Some(MacControlCoordinateSpace::ImagePixels)
        );
        assert_eq!(result.image_point.expect("image point").x, 200.0);
        let screen_point = result.screen_point.expect("screen point");
        assert_eq!(screen_point.x, 150.0);
        assert_eq!(screen_point.y, 145.0);
        assert_eq!(result.inside_frame, Some(true));
        assert_eq!(result.hit_elements[0].element.id, "el_1");
        assert!(result
            .hit_elements
            .iter()
            .any(|hit| hit.element.id == "el_2"));

        let action = result.suggested_action.expect("suggested click");
        assert_eq!(action.action, "act");
        assert_eq!(action.op, MacControlActOp::ClickPoint);
        assert_eq!(action.x, 150.0);
        assert_eq!(action.y, 145.0);
    }

    #[test]
    fn visual_point_maps_screen_points_and_allows_zero_pixel_origin() {
        let snapshot = sample_snapshot();
        let result = resolve_visual_point(
            &snapshot,
            MacControlCoordinateSpace::ScreenPoints,
            50.0,
            80.0,
            5,
        )
        .expect("visual point");

        assert_eq!(result.inside_frame, Some(true));
        let image_point = result.image_point.expect("image point");
        assert_eq!(image_point.x, 0.0);
        assert_eq!(image_point.y, 0.0);
        let screen_point = result.screen_point.expect("screen point");
        assert_eq!(screen_point.x, 50.0);
        assert_eq!(screen_point.y, 80.0);
        assert!(result.suggested_action.is_some());
        assert!(!result.nearest_elements.is_empty());
    }

    #[test]
    fn visual_point_reports_outside_frame_and_nearest_elements() {
        let snapshot = sample_snapshot();
        let result = resolve_visual_point(
            &snapshot,
            MacControlCoordinateSpace::ImagePixels,
            800.0,
            600.0,
            2,
        )
        .expect("visual point");

        assert_eq!(result.inside_frame, Some(false));
        assert!(result.suggested_action.is_none());
        assert!(result.hit_elements.is_empty());
        assert_eq!(result.nearest_elements.len(), 2);
        assert!(result.nearest_elements[0].distance_points > 0.0);
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn visual_point_prefers_smallest_hit_over_actionable_parent() {
        let snapshot = sample_snapshot();
        let result = resolve_visual_point(
            &snapshot,
            MacControlCoordinateSpace::ScreenPoints,
            150.0,
            110.0,
            5,
        )
        .expect("visual point");

        assert_eq!(result.hit_elements[0].element.id, "el_2");
        assert!(result
            .hit_elements
            .iter()
            .any(|hit| hit.element.id == "el_window"));
    }

    #[test]
    fn visual_point_requires_screenshot_metadata() {
        let mut snapshot = sample_snapshot();
        snapshot.screenshot = None;

        let error = resolve_visual_point(
            &snapshot,
            MacControlCoordinateSpace::ImagePixels,
            0.0,
            0.0,
            5,
        )
        .expect_err("missing screenshot should fail");

        assert!(error.contains("has no screenshot metadata"));
    }

    #[test]
    fn visual_ocr_maps_image_bounds_to_screen_points() {
        let snapshot = sample_snapshot();
        let blocks = resolve_ocr_text_blocks(
            &snapshot,
            vec![MacControlOcrRawTextBlock {
                text: " 保存 ".to_string(),
                confidence: 0.91,
                image_bounds: MacControlBounds {
                    x: 0.0,
                    y: 20.0,
                    width: 40.0,
                    height: 20.0,
                },
            }],
            None,
        )
        .expect("ocr blocks");

        assert_eq!(blocks.len(), 1);
        let block = &blocks[0];
        assert_eq!(block.id, "txt_1");
        assert_eq!(block.text, "保存");
        assert_eq!(block.image_point.x, 20.0);
        assert_eq!(block.image_point.y, 30.0);
        assert_eq!(block.screen_bounds.x, 50.0);
        assert_eq!(block.screen_bounds.y, 90.0);
        assert_eq!(block.screen_bounds.width, 20.0);
        assert_eq!(block.screen_bounds.height, 10.0);
        assert_eq!(block.screen_point.x, 60.0);
        assert_eq!(block.screen_point.y, 95.0);
    }

    #[test]
    fn visual_find_text_filters_and_returns_suggested_action() {
        let snapshot = sample_snapshot();
        let blocks = resolve_ocr_text_blocks(
            &snapshot,
            vec![
                MacControlOcrRawTextBlock {
                    text: "Open".to_string(),
                    confidence: 0.95,
                    image_bounds: MacControlBounds {
                        x: 140.0,
                        y: 90.0,
                        width: 80.0,
                        height: 30.0,
                    },
                },
                MacControlOcrRawTextBlock {
                    text: "Ignored".to_string(),
                    confidence: 0.1,
                    image_bounds: MacControlBounds {
                        x: 0.0,
                        y: 0.0,
                        width: 20.0,
                        height: 10.0,
                    },
                },
            ],
            Some(0.5),
        )
        .expect("ocr blocks");
        let matches = resolve_ocr_text_matches(
            &snapshot,
            &blocks,
            "pen",
            MacControlStringMatch::Contains,
            5,
        );

        assert_eq!(blocks.len(), 1);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].block.text, "Open");
        assert!(matches[0]
            .hit_elements
            .iter()
            .any(|hit| hit.element.id == "el_1"));
        let action = matches[0]
            .suggested_action
            .as_ref()
            .expect("suggested action");
        assert_eq!(action.op, MacControlActOp::ClickPoint);
        assert_eq!(action.x, matches[0].block.screen_point.x);
        assert_eq!(action.y, matches[0].block.screen_point.y);

        let none = resolve_ocr_text_matches(
            &snapshot,
            &blocks,
            "Cancel",
            MacControlStringMatch::Exact,
            5,
        );
        assert!(none.is_empty());
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
        assert_eq!(request.op, MacControlWaitOp::Present);
        assert!(!request.include_snapshot);
        let wait_with_snapshot: MacControlWaitRequest = serde_json::from_value(serde_json::json!({
            "op": "present",
            "includeSnapshot": true,
            "target": { "text": "Ready" }
        }))
        .expect("wait includeSnapshot request");
        assert!(wait_with_snapshot.include_snapshot);
    }

    #[test]
    fn wait_request_ignores_schema_filler_target_values() {
        let request = MacControlWaitRequest {
            target: MacControlTargetQuery {
                app_name: Some(" ".to_string()),
                bundle_id: Some("".to_string()),
                window_title: Some("".to_string()),
                window_title_match: MacControlStringMatch::Exact,
                element_id: Some("".to_string()),
                snapshot_id: Some("".to_string()),
                text: Some("".to_string()),
                role: Some("".to_string()),
                enabled: Some(false),
                focused: Some(false),
            },
            ..Default::default()
        }
        .clamped();

        assert!(request.target.is_empty());
        assert_eq!(request.target.snapshot_id, None);
        assert_eq!(request.target.enabled, None);
        assert_eq!(request.target.focused, None);
    }

    #[test]
    fn wait_gone_inverts_target_match_condition() {
        assert!(wait_condition_satisfied(MacControlWaitOp::Present, true));
        assert!(!wait_condition_satisfied(MacControlWaitOp::Present, false));
        assert!(!wait_condition_satisfied(MacControlWaitOp::Gone, true));
        assert!(wait_condition_satisfied(MacControlWaitOp::Gone, false));
        assert!(timeout_wait_message(MacControlWaitOp::Gone, 250).contains("disappear"));
    }

    #[test]
    fn apps_request_clamps_limit() {
        let request = MacControlAppsRequest {
            app_name: Some(" Finder ".to_string()),
            bundle_id: Some(" ".to_string()),
            pid: Some(0),
            limit: 10_000,
            ..Default::default()
        }
        .clamped();

        assert_eq!(request.limit, 100);
        assert_eq!(request.op, MacControlAppsOp::Frontmost);
        assert_eq!(request.app_name.as_deref(), Some("Finder"));
        assert_eq!(request.bundle_id, None);
        assert_eq!(request.pid, None);
    }

    #[test]
    fn activate_request_requires_target() {
        let missing = MacControlAppsRequest {
            op: MacControlAppsOp::Activate,
            app_name: Some("".to_string()),
            bundle_id: Some("".to_string()),
            pid: Some(0),
            ..Default::default()
        }
        .clamped();
        let by_name = MacControlAppsRequest {
            op: MacControlAppsOp::Activate,
            app_name: Some("Finder".to_string()),
            pid: Some(0),
            ..Default::default()
        }
        .clamped();
        let by_bundle = MacControlAppsRequest {
            op: MacControlAppsOp::Activate,
            bundle_id: Some("com.apple.finder".to_string()),
            pid: Some(0),
            ..Default::default()
        }
        .clamped();

        assert!(!apps_request_has_target(&missing));
        assert!(apps_request_has_target(&by_name));
        assert!(apps_request_has_target(&by_bundle));
        assert!(apps_launch_request_has_target(&by_name));
        assert!(apps_launch_request_has_target(&by_bundle));
        assert!(!apps_launch_request_has_target(&MacControlAppsRequest {
            op: MacControlAppsOp::Launch,
            pid: Some(123),
            ..Default::default()
        }));
        assert!(apps_request_has_target(&MacControlAppsRequest {
            op: MacControlAppsOp::Quit,
            pid: Some(123),
            ..Default::default()
        }));
    }

    #[test]
    fn focus_anchor_restore_requests_prefer_pid_then_bundle_then_name() {
        let anchor = MacControlFocusAnchor {
            pid: 42,
            bundle_id: Some("com.apple.TextEdit".to_string()),
            name: Some("TextEdit".to_string()),
            focused_window_id: None,
            focused_window_title: None,
        };

        let requests = focus_anchor_activate_requests(&anchor);
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].op, MacControlAppsOp::Activate);
        assert_eq!(requests[0].pid, Some(42));
        assert_eq!(requests[0].bundle_id, None);
        assert_eq!(requests[1].bundle_id.as_deref(), Some("com.apple.TextEdit"));
        assert_eq!(requests[1].pid, None);
        assert_eq!(requests[2].app_name.as_deref(), Some("TextEdit"));
        assert_eq!(requests[2].app_name_match, MacControlAppNameMatch::Exact);
    }

    #[test]
    fn focus_anchor_window_restore_requests_prefer_pid_scoped_id_then_title() {
        let anchor = MacControlFocusAnchor {
            pid: 42,
            bundle_id: Some("com.apple.TextEdit".to_string()),
            name: Some("TextEdit".to_string()),
            focused_window_id: Some("win_2".to_string()),
            focused_window_title: Some("Draft.txt".to_string()),
        };

        let requests = focus_anchor_window_requests(&anchor);
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].op, MacControlWindowsOp::Focus);
        assert_eq!(requests[0].window_scope, MacControlWindowsScope::All);
        assert_eq!(requests[0].window_id.as_deref(), Some("win_42_2"));
        assert_eq!(
            requests[0].target.window_title.as_deref(),
            Some("Draft.txt")
        );
        assert_eq!(
            requests[0].target.window_title_match,
            MacControlStringMatch::Exact
        );
        assert_eq!(requests[1].op, MacControlWindowsOp::Focus);
        assert_eq!(requests[1].window_scope, MacControlWindowsScope::Frontmost);
        assert_eq!(
            requests[1].target.window_title.as_deref(),
            Some("Draft.txt")
        );
        assert_eq!(
            requests[1].target.window_title_match,
            MacControlStringMatch::Exact
        );
    }

    #[test]
    fn dock_request_normalizes_targets_and_requires_launch_target() {
        let request = MacControlDockRequest {
            op: MacControlDockOp::Launch,
            dock_item_id: Some(" dock_1 ".to_string()),
            app_name: Some(" TextEdit ".to_string()),
            bundle_id: Some(" com.apple.TextEdit ".to_string()),
            item_path: Some(" /Applications/TextEdit.app ".to_string()),
            menu_item: Some(" Options ".to_string()),
            menu_index: Some(1),
            limit: 0,
            ..Default::default()
        }
        .clamped();

        assert_eq!(request.dock_item_id.as_deref(), Some("dock_1"));
        assert_eq!(request.app_name.as_deref(), Some("TextEdit"));
        assert_eq!(request.bundle_id.as_deref(), Some("com.apple.TextEdit"));
        assert_eq!(
            request.item_path.as_deref(),
            Some("/Applications/TextEdit.app")
        );
        assert_eq!(request.menu_item.as_deref(), Some("Options"));
        assert_eq!(request.menu_index, Some(1));
        assert_eq!(request.limit, 100);
        assert!(dock_request_has_target(&request));
        assert!(validate_dock_request(&request).is_none());
        assert!(!dock_request_has_target(&MacControlDockRequest {
            op: MacControlDockOp::Launch,
            ..Default::default()
        }));
        assert_eq!(
            validate_dock_request(&MacControlDockRequest {
                op: MacControlDockOp::SelectMenu,
                dock_item_id: Some("dock_1".to_string()),
                ..Default::default()
            })
            .as_deref(),
            Some("mac_control dock.select_menu requires menuItem or menuIndex.")
        );
    }

    #[test]
    fn sanitize_dock_select_menu_prefers_menu_item_over_index_noise() {
        let args = serde_json::json!({
            "action": "dock",
            "op": "select_menu",
            "bundleId": "com.apple.TextEdit",
            "menuItem": "Show in Finder",
            "menuIndex": 0
        });

        let sanitized = sanitize_tool_args(&args);
        assert!(sanitized.get("menuIndex").is_none());
        assert_eq!(
            sanitized.get("menuItem").and_then(|value| value.as_str()),
            Some("Show in Finder")
        );
        assert!(preflight_tool_args(&sanitized).is_none());

        let request: MacControlDockRequest = serde_json::from_value(sanitized).unwrap();
        let request = request.clamped();
        assert_eq!(request.menu_item.as_deref(), Some("Show in Finder"));
        assert_eq!(request.menu_index, None);
    }

    #[test]
    fn spaces_switch_requires_exactly_one_selector() {
        let none = MacControlSpacesRequest {
            op: MacControlSpacesOp::Switch,
            ..Default::default()
        }
        .clamped();
        assert!(validate_spaces_request(&none)
            .expect("validation error")
            .contains("exactly one"));

        let both = MacControlSpacesRequest {
            op: MacControlSpacesOp::Switch,
            space_index: Some(2),
            direction: Some(MacControlSpaceDirection::Right),
            ..Default::default()
        }
        .clamped();
        assert!(validate_spaces_request(&both)
            .expect("validation error")
            .contains("exactly one"));

        let valid = MacControlSpacesRequest {
            op: MacControlSpacesOp::Switch,
            space_index: Some(2),
            ..Default::default()
        }
        .clamped();
        assert!(validate_spaces_request(&valid).is_none());
    }

    #[test]
    fn sanitize_spaces_switch_prefers_non_default_direction() {
        let args = serde_json::json!({
            "action": "spaces",
            "op": "switch",
            "direction": "right",
            "spaceIndex": 1
        });

        let sanitized = sanitize_tool_args(&args);
        assert!(sanitized.get("spaceIndex").is_none());
        assert_eq!(
            sanitized.get("direction").and_then(|value| value.as_str()),
            Some("right")
        );
        assert!(preflight_tool_args(&sanitized).is_none());

        let request: MacControlSpacesRequest = serde_json::from_value(sanitized).unwrap();
        let request = request.clamped();
        assert_eq!(request.direction, Some(MacControlSpaceDirection::Right));
        assert_eq!(request.space_index, None);
    }

    #[test]
    fn sanitize_spaces_switch_prefers_exact_index_over_default_left_direction() {
        let args = serde_json::json!({
            "action": "spaces",
            "op": "switch",
            "direction": "left",
            "spaceIndex": 1,
            "spaceId": 0
        });

        let sanitized = sanitize_tool_args(&args);
        assert!(sanitized.get("direction").is_none());
        assert_eq!(
            sanitized.get("spaceIndex").and_then(|value| value.as_u64()),
            Some(1)
        );
        assert!(preflight_tool_args(&sanitized).is_none());

        let request: MacControlSpacesRequest = serde_json::from_value(sanitized).unwrap();
        let request = request.clamped();
        assert_eq!(request.direction, None);
        assert_eq!(request.space_index, Some(1));
        assert_eq!(request.space_id, None);
    }

    #[test]
    fn sanitize_spaces_switch_prefers_index_when_direction_is_default_noise() {
        let args = serde_json::json!({
            "action": "spaces",
            "op": "switch",
            "direction": "left",
            "spaceIndex": 2
        });

        let sanitized = sanitize_tool_args(&args);
        assert!(sanitized.get("direction").is_none());
        assert_eq!(
            sanitized.get("spaceIndex").and_then(|value| value.as_u64()),
            Some(2)
        );
        assert!(preflight_tool_args(&sanitized).is_none());

        let request: MacControlSpacesRequest = serde_json::from_value(sanitized).unwrap();
        let request = request.clamped();
        assert_eq!(request.direction, None);
        assert_eq!(request.space_index, Some(2));
    }

    #[test]
    fn sanitize_spaces_switch_prefers_index_over_default_right_direction() {
        let args = serde_json::json!({
            "action": "spaces",
            "op": "switch",
            "direction": "right",
            "spaceIndex": 2
        });

        let sanitized = sanitize_tool_args(&args);
        assert!(sanitized.get("direction").is_none());
        assert_eq!(
            sanitized.get("spaceIndex").and_then(|value| value.as_u64()),
            Some(2)
        );
        assert!(preflight_tool_args(&sanitized).is_none());

        let request: MacControlSpacesRequest = serde_json::from_value(sanitized).unwrap();
        let request = request.clamped();
        assert_eq!(request.direction, None);
        assert_eq!(request.space_index, Some(2));
    }

    #[test]
    fn preflight_spaces_switch_blocks_missing_selector_before_approval() {
        let args = serde_json::json!({
            "action": "spaces",
            "op": "switch"
        });

        let error = preflight_tool_args(&sanitize_tool_args(&args)).expect("preflight error");
        assert!(error.contains("exactly one"));
    }

    #[test]
    fn phase3_requests_normalize_and_validate() {
        let windows = MacControlWindowsRequest {
            window_scope: MacControlWindowsScope::All,
            target: MacControlTargetQuery {
                window_title: Some(" Notes ".to_string()),
                ..Default::default()
            },
            x: Some(10.0),
            y: Some(20.0),
            ..Default::default()
        }
        .clamped();
        assert!(windows_request_has_target(&windows));
        assert_eq!(windows.window_scope, MacControlWindowsScope::All);
        assert_eq!(windows.target.window_title.as_deref(), Some("Notes"));
        assert_eq!(
            MacControlWindowsRequest::default().window_scope,
            MacControlWindowsScope::Frontmost
        );
        let close_window = MacControlWindowsRequest {
            op: MacControlWindowsOp::Close,
            target: MacControlTargetQuery {
                window_title: Some(" Notes ".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
        .clamped();
        assert!(windows_request_has_target(&close_window));
        assert!(validate_windows_request(&close_window).is_none());

        let act = MacControlActRequest {
            op: MacControlActOp::Hotkey,
            key: Some(" ".to_string()),
            keys: vec![" cmd ".to_string(), "".to_string(), "n".to_string()],
            ..Default::default()
        }
        .clamped();
        assert_eq!(act.key, None);
        assert_eq!(act.keys, vec!["cmd".to_string(), "n".to_string()]);
        assert!(!act.include_snapshot);
        assert!(validate_act_request(&act).is_none());

        let act_with_snapshot: MacControlActRequest = serde_json::from_value(serde_json::json!({
            "op": "click",
            "includeSnapshot": true,
            "target": { "elementId": "el_20" }
        }))
        .expect("act includeSnapshot request");
        assert!(act_with_snapshot.include_snapshot);

        let paste_without_text = MacControlActRequest {
            op: MacControlActOp::Paste,
            ..Default::default()
        }
        .clamped();
        assert_eq!(
            validate_act_request(&paste_without_text).as_deref(),
            Some("mac_control act.paste requires text.")
        );

        let default_origin_click = MacControlActRequest {
            op: MacControlActOp::Click,
            x: Some(0.0),
            y: Some(0.0),
            ..Default::default()
        }
        .clamped();
        assert_eq!(default_origin_click.x, Some(0.0));
        assert_eq!(default_origin_click.y, Some(0.0));
        assert!(validate_act_request(&default_origin_click).is_some());

        let explicit_origin_click = MacControlActRequest {
            op: MacControlActOp::ClickPoint,
            x: Some(0.0),
            y: Some(0.0),
            ..Default::default()
        }
        .clamped();
        assert!(validate_act_request(&explicit_origin_click).is_none());

        let move_cursor = MacControlActRequest {
            op: MacControlActOp::MoveCursor,
            x: Some(0.0),
            y: Some(0.0),
            steps: Some(999),
            duration_ms: Some(99_999),
            motion_profile: Some(MacControlMotionProfile::Human),
            ..Default::default()
        }
        .clamped();
        assert_eq!(move_cursor.steps, Some(HARD_MOTION_STEPS));
        assert_eq!(move_cursor.duration_ms, Some(HARD_MOTION_DURATION_MS));
        assert_eq!(
            move_cursor.motion_profile,
            Some(MacControlMotionProfile::Human)
        );
        assert!(validate_act_request(&move_cursor).is_none());

        let ambiguous_move_cursor = MacControlActRequest {
            op: MacControlActOp::MoveCursor,
            target: MacControlTargetQuery {
                element_id: Some("el_20".to_string()),
                ..Default::default()
            },
            x: Some(0.0),
            y: Some(0.0),
            ..Default::default()
        }
        .clamped();
        assert!(validate_act_request(&ambiguous_move_cursor).is_some());

        let targeted_click = MacControlActRequest {
            op: MacControlActOp::Click,
            target: MacControlTargetQuery {
                element_id: Some(" el_20 ".to_string()),
                snapshot_id: Some(" macsnap_123 ".to_string()),
                ..Default::default()
            },
            x: Some(0.0),
            y: Some(0.0),
            ..Default::default()
        }
        .clamped();
        assert_eq!(targeted_click.target.element_id.as_deref(), Some("el_20"));
        assert_eq!(
            targeted_click.target.snapshot_id.as_deref(),
            Some("macsnap_123")
        );
        assert_eq!(targeted_click.x, Some(0.0));
        assert_eq!(targeted_click.y, Some(0.0));
        assert!(validate_act_request(&targeted_click).is_none());

        let dry_run = MacControlActRequest {
            op: MacControlActOp::DryRun,
            target: MacControlTargetQuery {
                text: Some(" Open ".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
        .clamped();
        assert_eq!(dry_run.target.text.as_deref(), Some("Open"));
        assert!(validate_act_request(&dry_run).is_none());

        let dry_run_without_target = MacControlActRequest {
            op: MacControlActOp::DryRun,
            ..Default::default()
        }
        .clamped();
        assert_eq!(
            validate_act_request(&dry_run_without_target).as_deref(),
            Some("mac_control act.dry_run requires a target.")
        );

        let perform_action = MacControlActRequest {
            op: MacControlActOp::PerformAction,
            ax_action: Some(" axshowmenu ".to_string()),
            target: MacControlTargetQuery {
                element_id: Some("el_20".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
        .clamped();
        assert_eq!(perform_action.ax_action.as_deref(), Some("axshowmenu"));
        assert_eq!(
            normalize_perform_ax_action(perform_action.ax_action.as_deref().expect("ax action")),
            Some("AXShowMenu".to_string())
        );
        assert!(validate_act_request(&perform_action).is_none());

        let perform_action_without_target = MacControlActRequest {
            op: MacControlActOp::PerformAction,
            ax_action: Some("AXPress".to_string()),
            ..Default::default()
        }
        .clamped();
        assert_eq!(
            validate_act_request(&perform_action_without_target).as_deref(),
            Some("mac_control act.perform_action requires a target.")
        );

        let perform_action_without_action = MacControlActRequest {
            op: MacControlActOp::PerformAction,
            target: MacControlTargetQuery {
                element_id: Some("el_20".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
        .clamped();
        assert_eq!(
            validate_act_request(&perform_action_without_action).as_deref(),
            Some("mac_control act.perform_action requires axAction.")
        );

        let custom_perform_action = MacControlActRequest {
            op: MacControlActOp::PerformAction,
            ax_action: Some("AXDelete".to_string()),
            target: MacControlTargetQuery {
                element_id: Some("el_20".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
        .clamped();
        assert_eq!(
            normalize_perform_ax_action(
                custom_perform_action
                    .ax_action
                    .as_deref()
                    .expect("ax action")
            ),
            Some("AXDelete".to_string())
        );
        assert!(validate_act_request(&custom_perform_action).is_none());

        let invalid_perform_action = MacControlActRequest {
            op: MacControlActOp::PerformAction,
            ax_action: Some("AX Delete".to_string()),
            target: MacControlTargetQuery {
                element_id: Some("el_20".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
        .clamped();
        assert!(validate_act_request(&invalid_perform_action)
            .expect("validation error")
            .contains("ASCII letters"));

        let ambiguous_click_point = MacControlActRequest {
            op: MacControlActOp::ClickPoint,
            target: MacControlTargetQuery {
                element_id: Some("el_20".to_string()),
                ..Default::default()
            },
            x: Some(0.0),
            y: Some(0.0),
            ..Default::default()
        }
        .clamped();
        assert!(validate_act_request(&ambiguous_click_point).is_some());

        let double_click = MacControlActRequest {
            op: MacControlActOp::DoubleClick,
            target: MacControlTargetQuery {
                text: Some("Open".to_string()),
                ..Default::default()
            },
            x: Some(0.0),
            y: Some(0.0),
            ..Default::default()
        }
        .clamped();
        assert!(validate_act_request(&double_click).is_none());

        let right_click_without_target = MacControlActRequest {
            op: MacControlActOp::RightClick,
            x: Some(10.0),
            y: Some(10.0),
            ..Default::default()
        }
        .clamped();
        assert!(validate_act_request(&right_click_without_target).is_some());

        let press = MacControlActRequest {
            op: MacControlActOp::Press,
            key: Some(" enter ".to_string()),
            modifiers: vec![" cmd ".to_string(), "".to_string()],
            repeat: Some(999),
            interval_ms: Some(99_999),
            hold_ms: Some(99_999),
            ..Default::default()
        }
        .clamped();
        assert_eq!(press.key.as_deref(), Some("enter"));
        assert_eq!(press.modifiers, vec!["cmd".to_string()]);
        assert_eq!(press.repeat, Some(HARD_PRESS_REPEAT));
        assert_eq!(press.interval_ms, Some(HARD_PRESS_INTERVAL_MS));
        assert_eq!(press.hold_ms, Some(HARD_PRESS_HOLD_MS));
        assert!(validate_act_request(&press).is_none());

        let press_without_key = MacControlActRequest {
            op: MacControlActOp::Press,
            ..Default::default()
        }
        .clamped();
        assert!(validate_act_request(&press_without_key).is_some());

        let drag = MacControlActRequest {
            op: MacControlActOp::Drag,
            target: MacControlTargetQuery {
                element_id: Some("el_20".to_string()),
                ..Default::default()
            },
            x: Some(100.0),
            y: Some(200.0),
            ..Default::default()
        }
        .clamped();
        assert!(validate_act_request(&drag).is_none());

        let point_to_target_drag = MacControlActRequest {
            op: MacControlActOp::Drag,
            from_x: Some(10.0),
            from_y: Some(20.0),
            to_target: MacControlTargetQuery {
                element_id: Some("el_21".to_string()),
                ..Default::default()
            },
            modifiers: vec!["shift".to_string()],
            ..Default::default()
        }
        .clamped();
        assert!(validate_act_request(&point_to_target_drag).is_none());

        let ambiguous_drag_destination = MacControlActRequest {
            op: MacControlActOp::Drag,
            from_x: Some(10.0),
            from_y: Some(20.0),
            x: Some(100.0),
            y: Some(200.0),
            to_x: Some(120.0),
            to_y: Some(220.0),
            ..Default::default()
        }
        .clamped();
        assert!(validate_act_request(&ambiguous_drag_destination).is_some());

        let swipe = MacControlActRequest {
            op: MacControlActOp::Swipe,
            x: Some(0.0),
            y: Some(0.0),
            delta_x: Some(50.0),
            ..Default::default()
        }
        .clamped();
        assert!(validate_act_request(&swipe).is_none());

        let swipe_to_target = MacControlActRequest {
            op: MacControlActOp::Swipe,
            from_x: Some(0.0),
            from_y: Some(0.0),
            to_target: MacControlTargetQuery {
                element_id: Some("el_21".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
        .clamped();
        assert!(validate_act_request(&swipe_to_target).is_none());

        let swipe_without_delta = MacControlActRequest {
            op: MacControlActOp::Swipe,
            target: MacControlTargetQuery {
                element_id: Some("el_20".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
        .clamped();
        assert!(validate_act_request(&swipe_without_delta).is_some());

        let ambiguous_swipe_destination = MacControlActRequest {
            op: MacControlActOp::Swipe,
            x: Some(0.0),
            y: Some(0.0),
            delta_x: Some(20.0),
            to_target: MacControlTargetQuery {
                element_id: Some("el_21".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
        .clamped();
        assert!(validate_act_request(&ambiguous_swipe_destination).is_some());

        let menu = MacControlMenuRequest {
            op: MacControlMenuOp::Click,
            scope: MacControlMenuScope::System,
            path: vec![" File ".to_string(), "".to_string(), "New".to_string()],
            menu_index: Some(2),
            verify: true,
            max_depth: 100,
            ..Default::default()
        }
        .clamped();
        assert_eq!(menu.scope, MacControlMenuScope::System);
        assert_eq!(menu.path, vec!["File".to_string(), "New".to_string()]);
        assert_eq!(menu.menu_index, Some(2));
        assert!(menu.verify);
        assert_eq!(menu.max_depth, 8);
        assert!(validate_menu_request(&menu).is_none());

        let menu_args = serde_json::json!({
            "action": "menu",
            "op": "click",
            "scope": "system",
            "path": ["File", "New"],
            "menuIndex": 0
        });
        let sanitized_menu = sanitize_tool_args(&menu_args);
        assert!(sanitized_menu.get("menuIndex").is_none());
        assert_eq!(
            sanitized_menu
                .get("path")
                .and_then(|value| value.as_array())
                .map(Vec::len),
            Some(2)
        );
        assert!(preflight_tool_args(&sanitized_menu).is_none());

        let menu_without_target = MacControlMenuRequest {
            op: MacControlMenuOp::Click,
            ..Default::default()
        }
        .clamped();
        assert_eq!(
            validate_menu_request(&menu_without_target).as_deref(),
            Some("mac_control menu.click requires path or menuIndex.")
        );

        let popover_menu = MacControlMenuRequest {
            op: MacControlMenuOp::Popover,
            app_hint: Some(" Control Center ".to_string()),
            limit: 100,
            languages: vec![" ".to_string(), "zh-Hans".to_string()],
            min_confidence: Some(2.0),
            ..Default::default()
        }
        .clamped();
        assert_eq!(popover_menu.app_hint.as_deref(), Some("Control Center"));
        assert_eq!(popover_menu.limit, HARD_MENU_POPOVER_LIMIT);
        assert_eq!(popover_menu.languages, vec!["zh-Hans".to_string()]);
        assert_eq!(popover_menu.min_confidence, Some(1.0));

        assert_eq!(
            MacControlMenuRequest::default().scope,
            MacControlMenuScope::App
        );

        let clipboard = MacControlClipboardRequest {
            op: MacControlClipboardOp::Get,
            max_chars: 0,
            ..Default::default()
        }
        .clamped();
        assert_eq!(clipboard.max_chars, DEFAULT_CLIPBOARD_MAX_CHARS);

        let big_clipboard = MacControlClipboardRequest {
            op: MacControlClipboardOp::Get,
            max_chars: 1_000_000,
            ..Default::default()
        }
        .clamped();
        assert_eq!(big_clipboard.max_chars, HARD_CLIPBOARD_MAX_CHARS);

        let oversized_set = MacControlClipboardRequest {
            op: MacControlClipboardOp::Set,
            text: Some("x".repeat(HARD_CLIPBOARD_SET_CHARS + 1)),
            ..Default::default()
        }
        .clamped();
        assert_eq!(
            oversized_set.text_original_len,
            Some(HARD_CLIPBOARD_SET_CHARS + 1)
        );
        assert!(oversized_set.text_truncated);
        assert_eq!(
            oversized_set.text.as_deref().map(str::len),
            Some(HARD_CLIPBOARD_SET_CHARS)
        );

        let reclamped_set = oversized_set.clamped();
        assert_eq!(
            reclamped_set.text_original_len,
            Some(HARD_CLIPBOARD_SET_CHARS + 1)
        );
        assert!(reclamped_set.text_truncated);

        assert!(validate_clipboard_request(&MacControlClipboardRequest {
            op: MacControlClipboardOp::Set,
            ..Default::default()
        })
        .is_some());

        let dialog = MacControlDialogRequest {
            op: MacControlDialogOp::Input,
            button_text: Some(" OK ".to_string()),
            text: Some(" hello ".to_string()),
            field: Some(" Name ".to_string()),
            file_path: Some(" /tmp ".to_string()),
            file_name: Some(" out.txt ".to_string()),
            select_button: Some(" Save ".to_string()),
            include_snapshot: true,
            max_elements: 10_000,
            max_depth: 100,
            ..Default::default()
        }
        .clamped();
        assert_eq!(dialog.button_text.as_deref(), Some("OK"));
        assert_eq!(dialog.text.as_deref(), Some("hello"));
        assert_eq!(dialog.field.as_deref(), Some("Name"));
        assert_eq!(dialog.file_path.as_deref(), Some("/tmp"));
        assert_eq!(dialog.file_name.as_deref(), Some("out.txt"));
        assert_eq!(dialog.select_button.as_deref(), Some("Save"));
        assert!(dialog.include_snapshot);
        assert_eq!(dialog.max_elements, HARD_SNAPSHOT_MAX_ELEMENTS);
        assert_eq!(dialog.max_depth, HARD_SNAPSHOT_MAX_DEPTH);
        assert!(validate_dialog_request(&dialog).is_none());
        assert!(validate_dialog_request(&MacControlDialogRequest {
            op: MacControlDialogOp::Click,
            ..Default::default()
        })
        .is_some());
        assert!(validate_dialog_request(&MacControlDialogRequest {
            op: MacControlDialogOp::File,
            ..Default::default()
        })
        .is_some());
        let dialog_aliases: MacControlDialogRequest = serde_json::from_value(serde_json::json!({
            "op": "file",
            "button": "Open",
            "path": "/tmp",
            "name": "report.pdf",
            "select": "default",
            "ensure_expanded": true
        }))
        .unwrap();
        let dialog_aliases = dialog_aliases.clamped();
        assert_eq!(dialog_aliases.button_text.as_deref(), Some("Open"));
        assert_eq!(dialog_aliases.file_path.as_deref(), Some("/tmp"));
        assert_eq!(dialog_aliases.file_name.as_deref(), Some("report.pdf"));
        assert_eq!(dialog_aliases.select_button.as_deref(), Some("default"));
        assert!(dialog_aliases.ensure_expanded);

        let elements = MacControlElementsRequest {
            target: MacControlTargetQuery {
                text: Some(" Search ".to_string()),
                enabled: Some(false),
                ..Default::default()
            },
            limit: 10_000,
            max_elements: 10_000,
            max_depth: 100,
            ..Default::default()
        }
        .clamped();
        assert_eq!(elements.op, MacControlElementsOp::Find);
        assert_eq!(elements.target.text.as_deref(), Some("Search"));
        assert_eq!(elements.target.enabled, None);
        assert_eq!(elements.limit, HARD_ELEMENTS_LIMIT);
        assert_eq!(elements.max_elements, HARD_SNAPSHOT_MAX_ELEMENTS);
        assert_eq!(elements.max_depth, HARD_SNAPSHOT_MAX_DEPTH);
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
        let response = unsupported_wait_response("no wait bridge", MacControlWaitOp::Gone, target);

        assert_eq!(response.status.readiness, MacControlReadiness::Unsupported);
        assert_eq!(response.op, MacControlWaitOp::Gone);
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
    fn unsupported_phase3_responses_have_consistent_shape() {
        let windows = unsupported_windows_response("no windows bridge");
        assert_eq!(windows.status.readiness, MacControlReadiness::Unsupported);
        assert!(windows.result.is_none());
        assert_eq!(windows.error.as_deref(), Some("no windows bridge"));

        let act = unsupported_act_response("no act bridge");
        assert_eq!(act.status.readiness, MacControlReadiness::Unsupported);
        assert!(act.result.is_none());
        assert_eq!(act.error.as_deref(), Some("no act bridge"));

        let elements = unsupported_elements_response("no elements bridge");
        assert_eq!(elements.status.readiness, MacControlReadiness::Unsupported);
        assert!(elements.result.is_none());
        assert_eq!(elements.error.as_deref(), Some("no elements bridge"));

        let menu = unsupported_menu_response("no menu bridge");
        assert_eq!(menu.status.readiness, MacControlReadiness::Unsupported);
        assert!(menu.result.is_none());
        assert_eq!(menu.error.as_deref(), Some("no menu bridge"));

        let clipboard = unsupported_clipboard_response("no clipboard bridge");
        assert_eq!(clipboard.status.readiness, MacControlReadiness::Unsupported);
        assert!(clipboard.result.is_none());
        assert_eq!(clipboard.error.as_deref(), Some("no clipboard bridge"));

        let dialog = unsupported_dialog_response("no dialog bridge");
        assert_eq!(dialog.status.readiness, MacControlReadiness::Unsupported);
        assert!(dialog.result.is_none());
        assert_eq!(dialog.error.as_deref(), Some("no dialog bridge"));

        let visual = unsupported_visual_response("no visual bridge");
        assert_eq!(visual.status.readiness, MacControlReadiness::Unsupported);
        assert!(visual.result.is_none());
        assert_eq!(visual.error.as_deref(), Some("no visual bridge"));
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
    fn diagnostics_request_clamps_limit() {
        let defaulted = MacControlDiagnosticsRequest {
            limit: 0,
            ..Default::default()
        }
        .clamped();
        assert_eq!(defaulted.limit, DEFAULT_DIAGNOSTICS_LIMIT);

        let capped = MacControlDiagnosticsRequest {
            limit: HARD_DIAGNOSTICS_LIMIT + 10,
            ..Default::default()
        }
        .clamped();
        assert_eq!(capped.limit, HARD_DIAGNOSTICS_LIMIT);
    }

    #[test]
    fn diagnostics_snapshot_summary_stays_compact() {
        let mut snapshot = MacControlSnapshot::new_empty();
        snapshot.snapshot_id = "macsnap_diag".to_string();
        snapshot.frontmost_app = Some(MacControlAppSummary {
            pid: 42,
            bundle_id: Some("com.example.App".to_string()),
            name: Some("Example".to_string()),
        });
        snapshot.displays.push(MacControlDisplaySummary {
            id: 1,
            frame_points: MacControlBounds {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 80.0,
            },
            scale: 2.0,
        });
        snapshot.windows.push(MacControlWindowSummary {
            id: "win_1".to_string(),
            app_pid: Some(42),
            role: Some("AXWindow".to_string()),
            subrole: None,
            title: Some("Main".to_string()),
            focused: true,
            bounds_points: None,
        });
        snapshot.elements.push(MacControlElementSummary {
            id: "el_1".to_string(),
            window_id: Some("win_1".to_string()),
            role: Some("AXButton".to_string()),
            label: Some("OK".to_string()),
            value: None,
            enabled: Some(true),
            focused: false,
            bounds_points: None,
            actions: vec!["AXPress".to_string()],
        });
        snapshot.screenshot = Some(MacControlScreenshotSummary {
            media_id: "macsnap_diag".to_string(),
            path: "/tmp/macsnap_diag.jpg".to_string(),
            width_px: 200,
            height_px: 160,
            target: MacControlScreenshotTarget::Display,
            display_id: Some(1),
            window_id: None,
            window_title: None,
            bounds_points: None,
            scale: Some(2.0),
        });
        snapshot.truncated = true;
        snapshot.warnings.push("truncated".to_string());

        let summary = cached_snapshot_summary(&snapshot);

        assert_eq!(summary.snapshot_id, "macsnap_diag");
        assert_eq!(summary.display_count, 1);
        assert_eq!(summary.window_count, 1);
        assert_eq!(summary.element_count, 1);
        assert!(summary.has_screenshot);
        assert!(summary.truncated);
        assert_eq!(summary.warnings, vec!["truncated".to_string()]);
    }

    #[test]
    fn runtime_stats_tracks_recent_errors() {
        record_error("test.op", "first");
        record_error("test.op", "first");
        let stats = runtime_stats();
        let entry = stats
            .recent_errors
            .iter()
            .find(|entry| entry.operation == "test.op" && entry.message == "first")
            .expect("recorded mac control error stat");
        assert!(entry.count >= 2);
        assert_eq!(stats.snapshot_cache_limit, MAX_SNAPSHOT_CACHE);
        assert_eq!(stats.screenshot_file_limit, MAX_SCREENSHOT_FILES);
    }

    #[test]
    fn target_query_matches_combined_app_window_and_element_filters() {
        let snapshot = sample_snapshot();
        let target = MacControlTargetQuery {
            app_name: Some("find".to_string()),
            window_title: Some("down".to_string()),
            window_title_match: MacControlStringMatch::Contains,
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
    fn target_window_title_match_defaults_to_exact() {
        let snapshot = sample_snapshot();
        let exact = MacControlTargetQuery {
            window_title: Some("Downloads".to_string()),
            ..Default::default()
        };
        let partial_default = MacControlTargetQuery {
            window_title: Some("down".to_string()),
            ..Default::default()
        };
        let partial_contains = MacControlTargetQuery {
            window_title: Some("down".to_string()),
            window_title_match: MacControlStringMatch::Contains,
            ..Default::default()
        };

        assert_eq!(find_target_matches(&snapshot, &exact).windows.len(), 1);
        assert!(find_target_matches(&snapshot, &partial_default)
            .windows
            .is_empty());
        assert_eq!(
            find_target_matches(&snapshot, &partial_contains)
                .windows
                .len(),
            1
        );
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
