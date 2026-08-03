//! System permission catalog, checks, and request entrypoints.
//!
//! v2 is intentionally macOS-first. Platforms without a real implementation
//! report `supported=false` instead of returning fake granted states.

use serde::Serialize;
use std::time::Duration;

/// Catalog-wide budget for one full check pass. Must absorb the worst-case
/// slow items IN SERIES: the screen-recording fresh-process probe (≤1.5s in
/// the platform layer) plus the notifications XPC query (≤2s) plus ~26 fast
/// items. 3s was breachable, and the timeout fallback degrades the whole
/// catalog to `unsupported_response()` — the panel would render the
/// "macOS-only" page on a real Mac.
const CHECK_TIMEOUT: Duration = Duration::from_secs(6);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(65);
/// `tccutil` is a short-lived local process; it either answers quickly or is
/// wedged, and the user is waiting on a button press.
const RESET_TIMEOUT: Duration = Duration::from_secs(15);

/// Stdout handshake prefix of the `--tcc-probe` process mode — the contract
/// between the spawning side (`platform/system_permissions.rs`) and the
/// answering side (src-tauri `main.rs`). The child prints
/// `{PREFIX}1|0|unknown`; the parent trusts ONLY this token, never the bare
/// exit code (a stale on-disk binary that predates the flag can exit 0 via
/// unrelated dispatch paths, e.g. a single-instance forward).
pub const TCC_PROBE_OUTPUT_PREFIX: &str = "hope-agent-tcc-probe:granted=";

// ── Public data types ────────────────────────────────────────────

/// Legacy v1 state: "granted" | "not_granted" | "unknown".
pub type PermState = String;

pub fn granted() -> PermState {
    "granted".into()
}

pub fn not_granted() -> PermState {
    "not_granted".into()
}

pub fn unknown() -> PermState {
    "unknown".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct PermissionStatus {
    pub id: String,
    pub status: PermState,
}

#[derive(Debug, Clone, Serialize)]
pub struct AllPermissions {
    pub accessibility: PermState,
    pub screen_recording: PermState,
    pub automation: PermState,
    pub app_management: PermState,
    pub full_disk_access: PermState,
    pub location: PermState,
    pub contacts: PermState,
    pub calendar: PermState,
    pub reminders: PermState,
    pub photos: PermState,
    pub camera: PermState,
    pub microphone: PermState,
    pub local_network: PermState,
    pub bluetooth: PermState,
    pub files_and_folders: PermState,
}

impl Default for AllPermissions {
    fn default() -> Self {
        Self {
            accessibility: unknown(),
            screen_recording: unknown(),
            automation: unknown(),
            app_management: unknown(),
            full_disk_access: unknown(),
            location: unknown(),
            contacts: unknown(),
            calendar: unknown(),
            reminders: unknown(),
            photos: unknown(),
            camera: unknown(),
            microphone: unknown(),
            local_network: unknown(),
            bluetooth: unknown(),
            files_and_folders: unknown(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemPermissionGroup {
    ControlCapture,
    FileAccess,
    PersonalData,
    DeviceNetwork,
    SystemServices,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemPermissionStatus {
    Granted,
    /// TCC has granted the permission, but this running process cannot use
    /// it yet — macOS fixes Screen Recording capability per window-server
    /// connection at launch, so the app must be relaunched to pick it up.
    GrantedPendingRestart,
    NotGranted,
    NotDetermined,
    Restricted,
    ManualCheck,
    NotApplicable,
    NotUsed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemPermissionRequestMode {
    NativePrompt,
    OpenSettings,
    TriggerProbe,
    None,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemPermissionItem {
    pub id: String,
    pub group: SystemPermissionGroup,
    pub status: SystemPermissionStatus,
    pub request_mode: SystemPermissionRequestMode,
    pub settings_pane: Option<String>,
    pub usage: String,
    pub note: Option<String>,
    /// True when `note` carries the post-request troubleshooting text rather
    /// than the static catalog note. The frontend keys its translation off
    /// this flag and keeps such notes pinned across refetches (a plain
    /// re-check cannot reproduce them).
    pub troubleshoot: bool,
    /// True when this build can reset the OS permission record for this item
    /// (macOS `tccutil`, packaged app only). Drives whether the panel offers
    /// the reset action — it is NOT a security boundary: `reset_system_permission`
    /// re-validates against the same whitelist.
    pub resettable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemPermissionsResponse {
    pub platform: String,
    pub supported: bool,
    pub items: Vec<SystemPermissionItem>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PermissionDef {
    pub(crate) id: &'static str,
    group: SystemPermissionGroup,
    request_mode: SystemPermissionRequestMode,
    pub(crate) settings_pane: Option<&'static str>,
    usage: &'static str,
    note: Option<&'static str>,
    /// Shown in place of `note` when a request attempt still ends up
    /// `NotGranted`. Lives on the def (not a separate id-match) so the
    /// pairing is compile-time visible next to the permission it describes.
    /// The frontend translates it under a distinct
    /// `permissionItems.<id>.troubleshootNote` key — it must never reuse the
    /// static `note` key, whose translations say something else.
    troubleshoot_note: Option<&'static str>,
}

impl PermissionDef {
    fn item(self, status: SystemPermissionStatus) -> SystemPermissionItem {
        SystemPermissionItem {
            id: self.id.to_string(),
            group: self.group,
            status,
            request_mode: self.request_mode,
            settings_pane: self.settings_pane.map(str::to_string),
            usage: self.usage.to_string(),
            note: self.note.map(str::to_string),
            troubleshoot: false,
            resettable: crate::platform::system_permission_supports_reset(self.id),
        }
    }

    /// Item shaped for a request response: swaps in the troubleshooting note
    /// when the attempt left the permission ungranted.
    fn request_item(self, status: SystemPermissionStatus) -> SystemPermissionItem {
        let mut item = self.item(status);
        if status == SystemPermissionStatus::NotGranted {
            if let Some(note) = self.troubleshoot_note {
                item.note = Some(note.to_string());
                item.troubleshoot = true;
            }
        }
        item
    }
}

/// Post-request troubleshooting hints, wired to their permission through
/// `PermissionDef::troubleshoot_note` and attached only when a request still
/// ends up NotGranted. English fallbacks — the frontend translates them via
/// `settings.permissionItems.<id>.troubleshootNote`. They exist because a TCC
/// record created by an old build (pre-stable-signing) keeps the System
/// Settings toggle visible yet permanently denies the current binary, which
/// is indistinguishable from "not granted" on our side.
const SCREEN_RECORDING_TROUBLESHOOT_NOTE: &str = "If System Settings already shows Hope Agent as allowed but it stays not granted here: restart the app; if that persists, remove Hope Agent from the Screen Recording list and grant it again (entries left by old versions go stale).";
const INPUT_MONITORING_TROUBLESHOOT_NOTE: &str = "If no system dialog appeared and Hope Agent is missing from the Input Monitoring list, add it manually in System Settings (or remove a stale entry) and try again.";

const PERMISSION_DEFS: &[PermissionDef] = &[
    PermissionDef {
        id: "accessibility",
        group: SystemPermissionGroup::ControlCapture,
        // NativePrompt: the request path calls the prompting AX trust check,
        // which is what registers the app in the Accessibility pane. A bare
        // OpenSettings jump lands the user on a list without any Hope Agent
        // row to toggle.
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_Accessibility"),
        usage: "Control the mouse, keyboard, and other application windows.",
        note: None,
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "screen_recording",
        group: SystemPermissionGroup::ControlCapture,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_ScreenCapture"),
        usage: "Capture screen contents for visual understanding and UI automation.",
        note: None,
        troubleshoot_note: Some(SCREEN_RECORDING_TROUBLESHOOT_NOTE),
    },
    PermissionDef {
        id: "system_audio_capture",
        group: SystemPermissionGroup::ControlCapture,
        request_mode: SystemPermissionRequestMode::None,
        settings_pane: Some("Privacy_AudioCapture"),
        usage: "Capture system audio when a future workflow explicitly needs it.",
        note: None,
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "input_monitoring",
        group: SystemPermissionGroup::ControlCapture,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_ListenEvent"),
        usage: "Listen for keyboard and pointer events needed by desktop automation.",
        note: None,
        troubleshoot_note: Some(INPUT_MONITORING_TROUBLESHOOT_NOTE),
    },
    PermissionDef {
        id: "automation_system_events",
        group: SystemPermissionGroup::ControlCapture,
        request_mode: SystemPermissionRequestMode::TriggerProbe,
        settings_pane: Some("Privacy_Automation"),
        usage: "Allow Apple Events automation of System Events.",
        note: Some("Per-target Automation consent is best confirmed in System Settings."),
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "automation_messages",
        group: SystemPermissionGroup::ControlCapture,
        request_mode: SystemPermissionRequestMode::TriggerProbe,
        settings_pane: Some("Privacy_Automation"),
        usage: "Allow Apple Events automation of Messages when messaging workflows need it.",
        note: Some("Per-target Automation consent is best confirmed in System Settings."),
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "app_management",
        group: SystemPermissionGroup::ControlCapture,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_AppBundles"),
        usage: "Manage or update other applications when a tool explicitly needs it.",
        note: Some("No reliable public per-app status API is available."),
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "developer_tools",
        group: SystemPermissionGroup::ControlCapture,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_DevTools"),
        usage: "Use developer tooling that macOS protects behind Developer Tools consent.",
        note: Some("No reliable public per-app status API is available."),
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "full_disk_access",
        group: SystemPermissionGroup::FileAccess,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_AllFiles"),
        usage: "Read protected files that normal Files & Folders consent does not cover.",
        note: Some("Detected with a conservative filesystem probe; absence is shown as manual confirmation."),
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "desktop_folder",
        group: SystemPermissionGroup::FileAccess,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_FilesAndFolders"),
        usage: "Read and write files on the Desktop when requested by the user.",
        note: Some("macOS exposes this through Files & Folders; status is probed."),
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "documents_folder",
        group: SystemPermissionGroup::FileAccess,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_FilesAndFolders"),
        usage: "Read and write files in Documents when requested by the user.",
        note: Some("macOS exposes this through Files & Folders; status is probed."),
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "downloads_folder",
        group: SystemPermissionGroup::FileAccess,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_FilesAndFolders"),
        usage: "Read and write files in Downloads when requested by the user.",
        note: Some("macOS exposes this through Files & Folders; status is probed."),
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "removable_volumes",
        group: SystemPermissionGroup::FileAccess,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_RemovableVolumes"),
        usage: "Access removable drives when the user asks the app to work there.",
        note: Some("No reliable public per-volume status API is available."),
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "network_volumes",
        group: SystemPermissionGroup::FileAccess,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_NetworkVolumes"),
        usage: "Access mounted network volumes when the user asks the app to work there.",
        note: Some("No reliable public per-volume status API is available."),
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "location",
        group: SystemPermissionGroup::PersonalData,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_LocationServices"),
        usage: "Use device location for local weather and location-aware workflows.",
        note: None,
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "contacts",
        group: SystemPermissionGroup::PersonalData,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_Contacts"),
        usage: "Read contacts only when a user workflow explicitly asks for it.",
        note: None,
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "calendar",
        group: SystemPermissionGroup::PersonalData,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_Calendars"),
        usage: "Read or write calendar events when scheduling workflows need it.",
        note: None,
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "reminders",
        group: SystemPermissionGroup::PersonalData,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_Reminders"),
        usage: "Read or write reminders when planning workflows need it.",
        note: None,
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "photos",
        group: SystemPermissionGroup::PersonalData,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_Photos"),
        usage: "Access the Photos library only when the user asks for photo workflows.",
        note: None,
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "media_library",
        group: SystemPermissionGroup::PersonalData,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_Media"),
        usage: "Access the media library only when a media workflow explicitly needs it.",
        note: Some("No reliable public status API is available for this app surface."),
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "speech_recognition",
        group: SystemPermissionGroup::PersonalData,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_SpeechRecognition"),
        usage: "Use speech recognition when a voice workflow asks for transcription.",
        note: None,
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "focus_status",
        group: SystemPermissionGroup::PersonalData,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_Focus"),
        usage: "Read Focus status only for workflows that adapt notifications or interruptions.",
        note: Some("No reliable public per-app status API is available."),
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "homekit",
        group: SystemPermissionGroup::PersonalData,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_HomeKit"),
        usage: "Access Home data only if future HomeKit workflows are enabled.",
        note: Some("Hope Agent does not currently use HomeKit workflows."),
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "camera",
        group: SystemPermissionGroup::DeviceNetwork,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_Camera"),
        usage: "Use the camera for visual input only when explicitly requested.",
        note: None,
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "microphone",
        group: SystemPermissionGroup::DeviceNetwork,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_Microphone"),
        usage: "Use the microphone for voice input only when explicitly requested.",
        note: None,
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "bluetooth",
        group: SystemPermissionGroup::DeviceNetwork,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_Bluetooth"),
        usage: "Discover and connect to Bluetooth devices when a workflow needs it.",
        note: None,
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "local_network",
        group: SystemPermissionGroup::DeviceNetwork,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_LocalNetwork"),
        usage: "Discover and connect to devices on the local network.",
        note: Some("macOS Local Network privacy has no reliable public status API."),
        troubleshoot_note: None,
    },
    PermissionDef {
        id: "notifications",
        group: SystemPermissionGroup::SystemServices,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Notifications"),
        usage: "Show system notifications. Delivery preferences remain in Notification settings.",
        note: Some("Notification configuration stays on the Notifications settings page."),
        troubleshoot_note: None,
    },
];

// ── Async helpers ────────────────────────────────────────────────

async fn blocking_with_timeout<T, F>(timeout: Duration, fallback: T, f: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    match tokio::time::timeout(timeout, tokio::task::spawn_blocking(f)).await {
        Ok(Ok(result)) => result,
        _ => fallback,
    }
}

// ── v2 API ───────────────────────────────────────────────────────

pub async fn check_system_permissions() -> SystemPermissionsResponse {
    blocking_with_timeout(CHECK_TIMEOUT, unsupported_response(), || {
        let supported = crate::platform::system_permissions_supported();
        let items = if supported {
            PERMISSION_DEFS
                .iter()
                .map(|def| def.item(crate::platform::check_system_permission_item(def.id)))
                .collect()
        } else {
            Vec::new()
        };
        SystemPermissionsResponse {
            platform: crate::platform::system_permissions_platform_name().to_string(),
            supported,
            items,
        }
    })
    .await
}

pub async fn request_system_permission(id: String) -> SystemPermissionItem {
    let fallback = unknown_item(id.clone());
    blocking_with_timeout(REQUEST_TIMEOUT, fallback, move || {
        let Some(def) = find_def(&id) else {
            return unknown_item(id);
        };
        let status = if crate::platform::system_permissions_supported() {
            crate::platform::request_system_permission_item(def)
        } else {
            SystemPermissionStatus::NotApplicable
        };
        def.request_item(status)
    })
    .await
}

/// Raw single-permission preflight backing the `--tcc-probe` process mode
/// (`hope-agent --tcc-probe <id>`): synchronous, no catalog shaping, and it
/// never spawns further probe processes.
pub fn raw_system_permission_probe(id: &str) -> Option<bool> {
    crate::platform::system_permission_raw_probe(id)
}

/// Drop this app's OS permission record for `id` so the OS asks again on the
/// next request (macOS `tccutil reset`). Remedies a record created by an older
/// build, which keeps the System Settings toggle visible while permanently
/// denying the current binary.
///
/// **Owner/GUI-only by design.** This is deliberately not a config field and
/// has no `ha-settings` category or model-facing tool: a model able to reset
/// TCC could strip the user's granted permissions at will, or farm fresh
/// consent prompts. It stays on the same footing as Provider credentials —
/// no agent surface, no new entry points.
pub async fn reset_system_permission(id: String) -> Result<SystemPermissionItem, String> {
    let Some(def) = find_def(&id) else {
        return Err(format!("Unknown permission id '{}'.", id));
    };
    if !crate::platform::system_permissions_supported() {
        return Err("System permissions are not supported on this platform.".to_string());
    }

    let reset_id = id.clone();
    let outcome = blocking_with_timeout(
        RESET_TIMEOUT,
        Err("Reset timed out.".to_string()),
        move || crate::platform::reset_system_permission_item(&reset_id),
    )
    .await;
    outcome?;

    // Re-check so the caller sees the post-reset state (typically
    // NotGranted/NotDetermined) instead of a stale pre-reset one.
    let status = blocking_with_timeout(CHECK_TIMEOUT, SystemPermissionStatus::NotGranted, {
        let id = id.clone();
        move || crate::platform::check_system_permission_item(&id)
    })
    .await;
    crate::app_info!("permissions", "reset", "{} → {:?}", id, status);
    Ok(def.item(status))
}

fn unsupported_response() -> SystemPermissionsResponse {
    SystemPermissionsResponse {
        platform: crate::platform::system_permissions_platform_name().to_string(),
        supported: false,
        items: Vec::new(),
    }
}

fn find_def(id: &str) -> Option<PermissionDef> {
    PERMISSION_DEFS.iter().copied().find(|def| def.id == id)
}

fn unknown_item(id: String) -> SystemPermissionItem {
    SystemPermissionItem {
        id,
        group: SystemPermissionGroup::SystemServices,
        status: SystemPermissionStatus::NotApplicable,
        request_mode: SystemPermissionRequestMode::None,
        settings_pane: None,
        usage: "This permission is not known by this version of Hope Agent.".to_string(),
        note: Some("Unknown permission id.".to_string()),
        troubleshoot: false,
        resettable: false,
    }
}

// ── v1 compatibility wrappers ────────────────────────────────────

pub async fn check_all_permissions() -> AllPermissions {
    let response = check_system_permissions().await;
    let legacy = legacy_from_response(&response);
    crate::app_info!(
        "permissions",
        "check_all",
        "platform={} supported={} a11y={} screen={} auto={} appmgmt={} fda={} loc={} contacts={} cal={} remind={} photos={} cam={} mic={} net={} bt={} files={}",
        response.platform,
        response.supported,
        legacy.accessibility,
        legacy.screen_recording,
        legacy.automation,
        legacy.app_management,
        legacy.full_disk_access,
        legacy.location,
        legacy.contacts,
        legacy.calendar,
        legacy.reminders,
        legacy.photos,
        legacy.camera,
        legacy.microphone,
        legacy.local_network,
        legacy.bluetooth,
        legacy.files_and_folders
    );
    legacy
}

pub async fn check_permission(id: String) -> PermissionStatus {
    let response = check_system_permissions().await;
    let status = legacy_status_for_id(&response, &id);
    PermissionStatus { id, status }
}

pub async fn request_permission(id: String) -> PermissionStatus {
    crate::app_info!("permissions", "request", "Requesting: {}", id);
    if let Some(request_id) = legacy_request_id(&id) {
        let _ = request_system_permission(request_id.to_string()).await;
    }
    let response = check_system_permissions().await;
    let status = legacy_status_for_id(&response, &id);
    crate::app_info!("permissions", "request", "{} → {}", id, status);
    PermissionStatus { id, status }
}

fn legacy_request_id(id: &str) -> Option<&str> {
    match id {
        "automation" => Some("automation_system_events"),
        "files_and_folders" => Some("desktop_folder"),
        "accessibility" | "screen_recording" | "app_management" | "full_disk_access"
        | "location" | "contacts" | "calendar" | "reminders" | "photos" | "camera"
        | "microphone" | "local_network" | "bluetooth" => Some(id),
        _ => None,
    }
}

fn legacy_status_for_id(response: &SystemPermissionsResponse, id: &str) -> PermState {
    match id {
        "automation" => legacy_item(response, "automation_system_events"),
        "files_and_folders" => legacy_files_and_folders(response),
        id => legacy_item(response, id),
    }
}

fn legacy_from_response(response: &SystemPermissionsResponse) -> AllPermissions {
    if !response.supported {
        return AllPermissions::default();
    }

    AllPermissions {
        accessibility: legacy_item(response, "accessibility"),
        screen_recording: legacy_item(response, "screen_recording"),
        automation: legacy_item(response, "automation_system_events"),
        app_management: legacy_item(response, "app_management"),
        full_disk_access: legacy_item(response, "full_disk_access"),
        location: legacy_item(response, "location"),
        contacts: legacy_item(response, "contacts"),
        calendar: legacy_item(response, "calendar"),
        reminders: legacy_item(response, "reminders"),
        photos: legacy_item(response, "photos"),
        camera: legacy_item(response, "camera"),
        microphone: legacy_item(response, "microphone"),
        local_network: legacy_item(response, "local_network"),
        bluetooth: legacy_item(response, "bluetooth"),
        files_and_folders: legacy_files_and_folders(response),
    }
}

fn legacy_item(response: &SystemPermissionsResponse, id: &str) -> PermState {
    response
        .items
        .iter()
        .find(|item| item.id == id)
        .map(|item| legacy_state_for_status(item.status))
        .unwrap_or_else(unknown)
}

fn legacy_files_and_folders(response: &SystemPermissionsResponse) -> PermState {
    let statuses = ["desktop_folder", "documents_folder", "downloads_folder"]
        .iter()
        .filter_map(|id| response.items.iter().find(|item| item.id == *id))
        .map(|item| item.status)
        .collect::<Vec<_>>();

    if statuses.len() == 3
        && statuses
            .iter()
            .all(|status| *status == SystemPermissionStatus::Granted)
    {
        granted()
    } else if statuses.iter().any(|status| {
        matches!(
            status,
            SystemPermissionStatus::NotGranted
                | SystemPermissionStatus::NotDetermined
                | SystemPermissionStatus::Restricted
                // Mirrors `legacy_state_for_status`: unusable by this
                // process, so it must not read as `unknown` here either.
                | SystemPermissionStatus::GrantedPendingRestart
        )
    }) {
        not_granted()
    } else {
        unknown()
    }
}

fn legacy_state_for_status(status: SystemPermissionStatus) -> PermState {
    match status {
        SystemPermissionStatus::Granted => granted(),
        // Pending-restart means this process still cannot use the permission,
        // so every capability gate must keep treating it as not granted.
        SystemPermissionStatus::GrantedPendingRestart
        | SystemPermissionStatus::NotGranted
        | SystemPermissionStatus::NotDetermined
        | SystemPermissionStatus::Restricted => not_granted(),
        SystemPermissionStatus::ManualCheck
        | SystemPermissionStatus::NotApplicable
        | SystemPermissionStatus::NotUsed => unknown(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reset_rejects_unknown_permission_id() {
        // The id comes from the UI; it must be validated against the catalog
        // before any platform call, so no caller can steer the reset at an
        // arbitrary target.
        let err = reset_system_permission("../../etc".to_string())
            .await
            .expect_err("unknown id must be rejected");
        assert!(err.contains("Unknown permission id"));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn reset_rejects_permissions_outside_the_whitelist() {
        // A real catalog id that has no tccutil service mapping: known to the
        // catalog, still not resettable.
        let err = reset_system_permission("full_disk_access".to_string())
            .await
            .expect_err("non-whitelisted permission must be rejected");
        assert!(
            err.contains("does not support reset") || err.contains("packaged app"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn legacy_manual_check_maps_to_unknown() {
        assert_eq!(
            legacy_state_for_status(SystemPermissionStatus::ManualCheck),
            "unknown"
        );
    }

    #[test]
    fn legacy_actionable_statuses_map_to_not_granted() {
        assert_eq!(
            legacy_state_for_status(SystemPermissionStatus::NotDetermined),
            "not_granted"
        );
        assert_eq!(
            legacy_state_for_status(SystemPermissionStatus::Restricted),
            "not_granted"
        );
        // A grant pending relaunch is unusable by this process, so legacy
        // consumers (mac_control gates, automation preflights) must not see
        // it as granted.
        assert_eq!(
            legacy_state_for_status(SystemPermissionStatus::GrantedPendingRestart),
            "not_granted"
        );
    }

    #[test]
    fn legacy_permission_ids_map_to_v2_items() {
        let response = SystemPermissionsResponse {
            platform: "macos".to_string(),
            supported: true,
            items: vec![
                PermissionDef {
                    id: "automation_system_events",
                    group: SystemPermissionGroup::ControlCapture,
                    request_mode: SystemPermissionRequestMode::TriggerProbe,
                    settings_pane: None,
                    usage: "",
                    note: None,
                    troubleshoot_note: None,
                }
                .item(SystemPermissionStatus::Granted),
                PermissionDef {
                    id: "desktop_folder",
                    group: SystemPermissionGroup::FileAccess,
                    request_mode: SystemPermissionRequestMode::OpenSettings,
                    settings_pane: None,
                    usage: "",
                    note: None,
                    troubleshoot_note: None,
                }
                .item(SystemPermissionStatus::Granted),
                PermissionDef {
                    id: "documents_folder",
                    group: SystemPermissionGroup::FileAccess,
                    request_mode: SystemPermissionRequestMode::OpenSettings,
                    settings_pane: None,
                    usage: "",
                    note: None,
                    troubleshoot_note: None,
                }
                .item(SystemPermissionStatus::Granted),
                PermissionDef {
                    id: "downloads_folder",
                    group: SystemPermissionGroup::FileAccess,
                    request_mode: SystemPermissionRequestMode::OpenSettings,
                    settings_pane: None,
                    usage: "",
                    note: None,
                    troubleshoot_note: None,
                }
                .item(SystemPermissionStatus::Granted),
            ],
        };

        assert_eq!(legacy_status_for_id(&response, "automation"), "granted");
        assert_eq!(
            legacy_status_for_id(&response, "files_and_folders"),
            "granted"
        );
        assert_eq!(
            legacy_request_id("automation"),
            Some("automation_system_events")
        );
        assert_eq!(
            legacy_request_id("files_and_folders"),
            Some("desktop_folder")
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[tokio::test]
    async fn non_macos_system_permissions_are_not_fake_granted() {
        let response = check_system_permissions().await;
        assert!(!response.supported);
        assert!(response.items.is_empty());

        let legacy = check_all_permissions().await;
        assert!(std::iter::once(&legacy.accessibility)
            .chain(std::iter::once(&legacy.screen_recording))
            .chain(std::iter::once(&legacy.automation))
            .chain(std::iter::once(&legacy.app_management))
            .chain(std::iter::once(&legacy.full_disk_access))
            .chain(std::iter::once(&legacy.location))
            .chain(std::iter::once(&legacy.contacts))
            .chain(std::iter::once(&legacy.calendar))
            .chain(std::iter::once(&legacy.reminders))
            .chain(std::iter::once(&legacy.photos))
            .chain(std::iter::once(&legacy.camera))
            .chain(std::iter::once(&legacy.microphone))
            .chain(std::iter::once(&legacy.local_network))
            .chain(std::iter::once(&legacy.bluetooth))
            .chain(std::iter::once(&legacy.files_and_folders))
            .all(|status| status == "unknown"));
    }
}
