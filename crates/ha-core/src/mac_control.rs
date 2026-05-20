//! macOS desktop control bridge and readiness model.
//!
//! Exposes status / permissions, Accessibility snapshots, scored element
//! search, display/window screenshot frames, wait/target matching, app
//! focus/launch, window operations, AX-first element actions, clipboard text,
//! dialogs, and menu inspection/clicks.

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
    pub target: Option<MacControlElementSummary>,
    pub snapshot: Option<MacControlSnapshot>,
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
pub struct MacControlMenuResult {
    pub op: MacControlMenuOp,
    pub scope: MacControlMenuScope,
    pub path: Vec<String>,
    pub items: Vec<MacControlMenuItemSummary>,
    pub clicked: Option<MacControlMenuItemSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlMenuItemSummary {
    pub title: Option<String>,
    pub description: Option<String>,
    pub value: Option<String>,
    pub role: Option<String>,
    pub enabled: Option<bool>,
    pub actions: Vec<String>,
    pub children: Vec<MacControlMenuItemSummary>,
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
    pub snapshot: Option<MacControlSnapshot>,
    pub execution: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlDialogSummary {
    pub window: MacControlWindowSummary,
    pub text: Vec<String>,
    pub buttons: Vec<MacControlElementSummary>,
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
    pub text: Option<String>,
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
    #[serde(default = "default_snapshot_max_elements")]
    pub max_elements: usize,
    #[serde(default = "default_snapshot_max_depth")]
    pub max_depth: usize,
}

impl MacControlActRequest {
    pub fn clamped(mut self) -> Self {
        self.target = self.target.normalized();
        self.text = normalize_optional_string(self.text);
        self.value = normalize_optional_string(self.value);
        self.key = normalize_optional_string(self.key);
        self.keys = self
            .keys
            .into_iter()
            .filter_map(|value| normalize_optional_string(Some(value)))
            .collect();
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
pub enum MacControlActOp {
    #[default]
    Click,
    DryRun,
    ClickPoint,
    DoubleClick,
    RightClick,
    Type,
    Paste,
    SetValue,
    Hotkey,
    Scroll,
    Drag,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlMenuRequest {
    #[serde(default)]
    pub op: MacControlMenuOp,
    #[serde(default)]
    pub scope: MacControlMenuScope,
    #[serde(default)]
    pub path: Vec<String>,
    #[serde(default = "default_menu_max_depth")]
    pub max_depth: usize,
}

impl MacControlMenuRequest {
    pub fn clamped(mut self) -> Self {
        self.path = self
            .path
            .into_iter()
            .filter_map(|value| normalize_optional_string(Some(value)))
            .collect();
        if self.max_depth == 0 {
            self.max_depth = default_menu_max_depth();
        }
        self.max_depth = self.max_depth.min(8);
        self
    }
}

fn default_menu_max_depth() -> usize {
    3
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacControlMenuOp {
    #[default]
    List,
    Click,
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
}

impl MacControlClipboardRequest {
    pub fn clamped(mut self) -> Self {
        if self.max_chars == 0 {
            self.max_chars = DEFAULT_CLIPBOARD_MAX_CHARS;
        }
        self.max_chars = self.max_chars.min(HARD_CLIPBOARD_MAX_CHARS);
        if let Some(text) = self.text.as_mut() {
            if text.chars().count() > HARD_CLIPBOARD_SET_CHARS {
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
    #[serde(default)]
    pub button_text: Option<String>,
    #[serde(default = "default_snapshot_max_elements")]
    pub max_elements: usize,
    #[serde(default = "default_snapshot_max_depth")]
    pub max_depth: usize,
}

impl MacControlDialogRequest {
    pub fn clamped(mut self) -> Self {
        self.target = self.target.normalized();
        self.button_text = normalize_optional_string(self.button_text);
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
                        snapshot: Some(snapshot),
                        error: None,
                    };
                }
                last_matches = matches;
                last_snapshot = Some(snapshot);
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
    if request.op == MacControlMenuOp::Click && request.path.is_empty() {
        return MacControlMenuResponse {
            status,
            result: None,
            error: Some("mac_control menu.click requires a non-empty path.".to_string()),
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
        MacControlActOp::ClickPoint => {
            if request.x.is_none() || request.y.is_none() {
                return Some("mac_control act.click_point requires x and y.".to_string());
            }
            if !request.target.is_empty() {
                return Some("mac_control act.click_point does not accept target; use act.click for AX element targets.".to_string());
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
        MacControlActOp::Scroll => {
            if request.delta_x.unwrap_or(0.0) == 0.0 && request.delta_y.unwrap_or(0.0) == 0.0 {
                return Some("mac_control act.scroll requires deltaX or deltaY.".to_string());
            }
        }
        MacControlActOp::Drag => {
            if request.target.is_empty() {
                return Some(
                    "mac_control act.drag requires a source target; x/y are the destination point."
                        .to_string(),
                );
            }
            if request.x.is_none() || request.y.is_none() {
                return Some(
                    "mac_control act.drag requires destination x and y in macOS screen points."
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
        MacControlActOp::ClickPoint => "click_point",
        MacControlActOp::DoubleClick => "double_click",
        MacControlActOp::RightClick => "right_click",
        MacControlActOp::Type => "type",
        MacControlActOp::Paste => "paste",
        MacControlActOp::SetValue => "set_value",
        MacControlActOp::Hotkey => "hotkey",
        MacControlActOp::Scroll => "scroll",
        MacControlActOp::Drag => "drag",
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
                text: Some("".to_string()),
                role: Some("".to_string()),
                enabled: Some(false),
                focused: Some(false),
            },
            ..Default::default()
        }
        .clamped();

        assert!(request.target.is_empty());
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
        assert!(validate_act_request(&act).is_none());

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

        let targeted_click = MacControlActRequest {
            op: MacControlActOp::Click,
            target: MacControlTargetQuery {
                element_id: Some(" el_20 ".to_string()),
                ..Default::default()
            },
            x: Some(0.0),
            y: Some(0.0),
            ..Default::default()
        }
        .clamped();
        assert_eq!(targeted_click.target.element_id.as_deref(), Some("el_20"));
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

        let menu = MacControlMenuRequest {
            op: MacControlMenuOp::Click,
            scope: MacControlMenuScope::System,
            path: vec![" File ".to_string(), "".to_string(), "New".to_string()],
            max_depth: 100,
        }
        .clamped();
        assert_eq!(menu.scope, MacControlMenuScope::System);
        assert_eq!(menu.path, vec!["File".to_string(), "New".to_string()]);
        assert_eq!(menu.max_depth, 8);

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
        assert!(validate_clipboard_request(&MacControlClipboardRequest {
            op: MacControlClipboardOp::Set,
            ..Default::default()
        })
        .is_some());

        let dialog = MacControlDialogRequest {
            op: MacControlDialogOp::Accept,
            button_text: Some(" OK ".to_string()),
            max_elements: 10_000,
            max_depth: 100,
            ..Default::default()
        }
        .clamped();
        assert_eq!(dialog.button_text.as_deref(), Some("OK"));
        assert_eq!(dialog.max_elements, HARD_SNAPSHOT_MAX_ELEMENTS);
        assert_eq!(dialog.max_depth, HARD_SNAPSHOT_MAX_DEPTH);

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
