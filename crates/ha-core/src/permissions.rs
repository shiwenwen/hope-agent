//! System permission catalog, checks, and request entrypoints.
//!
//! v2 is intentionally macOS-first. Platforms without a real implementation
//! report `supported=false` instead of returning fake granted states.

use serde::Serialize;
use std::time::Duration;

const CHECK_TIMEOUT: Duration = Duration::from_secs(3);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(65);

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
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemPermissionsResponse {
    pub platform: String,
    pub supported: bool,
    pub items: Vec<SystemPermissionItem>,
}

#[derive(Debug, Clone, Copy)]
struct PermissionDef {
    id: &'static str,
    group: SystemPermissionGroup,
    request_mode: SystemPermissionRequestMode,
    settings_pane: Option<&'static str>,
    usage: &'static str,
    note: Option<&'static str>,
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
        }
    }
}

const PERMISSION_DEFS: &[PermissionDef] = &[
    PermissionDef {
        id: "accessibility",
        group: SystemPermissionGroup::ControlCapture,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_Accessibility"),
        usage: "Control the mouse, keyboard, and other application windows.",
        note: None,
    },
    PermissionDef {
        id: "screen_recording",
        group: SystemPermissionGroup::ControlCapture,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_ScreenCapture"),
        usage: "Capture screen contents for visual understanding and UI automation.",
        note: None,
    },
    PermissionDef {
        id: "system_audio_capture",
        group: SystemPermissionGroup::ControlCapture,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_AudioCapture"),
        usage: "Capture system audio when a future workflow explicitly needs it.",
        note: Some("macOS does not expose a reliable public preflight API for this permission."),
    },
    PermissionDef {
        id: "input_monitoring",
        group: SystemPermissionGroup::ControlCapture,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_ListenEvent"),
        usage: "Listen for keyboard and pointer events needed by desktop automation.",
        note: None,
    },
    PermissionDef {
        id: "automation_system_events",
        group: SystemPermissionGroup::ControlCapture,
        request_mode: SystemPermissionRequestMode::TriggerProbe,
        settings_pane: Some("Privacy_Automation"),
        usage: "Allow Apple Events automation of System Events.",
        note: Some("Per-target Automation consent is best confirmed in System Settings."),
    },
    PermissionDef {
        id: "automation_messages",
        group: SystemPermissionGroup::ControlCapture,
        request_mode: SystemPermissionRequestMode::TriggerProbe,
        settings_pane: Some("Privacy_Automation"),
        usage: "Allow Apple Events automation of Messages when messaging workflows need it.",
        note: Some("Per-target Automation consent is best confirmed in System Settings."),
    },
    PermissionDef {
        id: "app_management",
        group: SystemPermissionGroup::ControlCapture,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_AppBundles"),
        usage: "Manage or update other applications when a tool explicitly needs it.",
        note: Some("No reliable public per-app status API is available."),
    },
    PermissionDef {
        id: "developer_tools",
        group: SystemPermissionGroup::ControlCapture,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_DevTools"),
        usage: "Use developer tooling that macOS protects behind Developer Tools consent.",
        note: Some("No reliable public per-app status API is available."),
    },
    PermissionDef {
        id: "full_disk_access",
        group: SystemPermissionGroup::FileAccess,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_AllFiles"),
        usage: "Read protected files that normal Files & Folders consent does not cover.",
        note: Some("Detected with a conservative filesystem probe; absence is shown as manual confirmation."),
    },
    PermissionDef {
        id: "desktop_folder",
        group: SystemPermissionGroup::FileAccess,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_FilesAndFolders"),
        usage: "Read and write files on the Desktop when requested by the user.",
        note: Some("macOS exposes this through Files & Folders; status is probed."),
    },
    PermissionDef {
        id: "documents_folder",
        group: SystemPermissionGroup::FileAccess,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_FilesAndFolders"),
        usage: "Read and write files in Documents when requested by the user.",
        note: Some("macOS exposes this through Files & Folders; status is probed."),
    },
    PermissionDef {
        id: "downloads_folder",
        group: SystemPermissionGroup::FileAccess,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_FilesAndFolders"),
        usage: "Read and write files in Downloads when requested by the user.",
        note: Some("macOS exposes this through Files & Folders; status is probed."),
    },
    PermissionDef {
        id: "removable_volumes",
        group: SystemPermissionGroup::FileAccess,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_RemovableVolumes"),
        usage: "Access removable drives when the user asks the app to work there.",
        note: Some("No reliable public per-volume status API is available."),
    },
    PermissionDef {
        id: "network_volumes",
        group: SystemPermissionGroup::FileAccess,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_NetworkVolumes"),
        usage: "Access mounted network volumes when the user asks the app to work there.",
        note: Some("No reliable public per-volume status API is available."),
    },
    PermissionDef {
        id: "location",
        group: SystemPermissionGroup::PersonalData,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_LocationServices"),
        usage: "Use device location for local weather and location-aware workflows.",
        note: None,
    },
    PermissionDef {
        id: "contacts",
        group: SystemPermissionGroup::PersonalData,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_Contacts"),
        usage: "Read contacts only when a user workflow explicitly asks for it.",
        note: None,
    },
    PermissionDef {
        id: "calendar",
        group: SystemPermissionGroup::PersonalData,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_Calendars"),
        usage: "Read or write calendar events when scheduling workflows need it.",
        note: None,
    },
    PermissionDef {
        id: "reminders",
        group: SystemPermissionGroup::PersonalData,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_Reminders"),
        usage: "Read or write reminders when planning workflows need it.",
        note: None,
    },
    PermissionDef {
        id: "photos",
        group: SystemPermissionGroup::PersonalData,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_Photos"),
        usage: "Access the Photos library only when the user asks for photo workflows.",
        note: None,
    },
    PermissionDef {
        id: "media_library",
        group: SystemPermissionGroup::PersonalData,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_Media"),
        usage: "Access the media library only when a media workflow explicitly needs it.",
        note: Some("No reliable public status API is available for this app surface."),
    },
    PermissionDef {
        id: "speech_recognition",
        group: SystemPermissionGroup::PersonalData,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_SpeechRecognition"),
        usage: "Use speech recognition when a voice workflow asks for transcription.",
        note: None,
    },
    PermissionDef {
        id: "focus_status",
        group: SystemPermissionGroup::PersonalData,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_Focus"),
        usage: "Read Focus status only for workflows that adapt notifications or interruptions.",
        note: Some("No reliable public per-app status API is available."),
    },
    PermissionDef {
        id: "homekit",
        group: SystemPermissionGroup::PersonalData,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_HomeKit"),
        usage: "Access Home data only if future HomeKit workflows are enabled.",
        note: Some("Hope Agent does not currently use HomeKit workflows."),
    },
    PermissionDef {
        id: "camera",
        group: SystemPermissionGroup::DeviceNetwork,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_Camera"),
        usage: "Use the camera for visual input only when explicitly requested.",
        note: None,
    },
    PermissionDef {
        id: "microphone",
        group: SystemPermissionGroup::DeviceNetwork,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_Microphone"),
        usage: "Use the microphone for voice input only when explicitly requested.",
        note: None,
    },
    PermissionDef {
        id: "bluetooth",
        group: SystemPermissionGroup::DeviceNetwork,
        request_mode: SystemPermissionRequestMode::NativePrompt,
        settings_pane: Some("Privacy_Bluetooth"),
        usage: "Discover and connect to Bluetooth devices when a workflow needs it.",
        note: None,
    },
    PermissionDef {
        id: "local_network",
        group: SystemPermissionGroup::DeviceNetwork,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Privacy_LocalNetwork"),
        usage: "Discover and connect to devices on the local network.",
        note: Some("macOS Local Network privacy has no reliable public status API."),
    },
    PermissionDef {
        id: "notifications",
        group: SystemPermissionGroup::SystemServices,
        request_mode: SystemPermissionRequestMode::OpenSettings,
        settings_pane: Some("Notifications"),
        usage: "Show system notifications. Delivery preferences remain in Notification settings.",
        note: Some("Notification configuration stays on the Notifications settings page."),
    },
];

// ── Platform-specific implementation ─────────────────────────────

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::ffi::CStr;
    use std::path::Path;
    use std::process::Command;
    use std::ptr;
    use std::sync::mpsc;

    use block2::RcBlock;
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2_foundation::NSString;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
        fn CGPreflightListenEventAccess() -> bool;
        fn CGRequestListenEventAccess() -> bool;
    }

    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "CoreBluetooth", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "CoreLocation", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "Contacts", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "EventKit", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "Photos", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "Speech", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "UserNotifications", kind = "framework")]
    unsafe extern "C" {}

    pub fn platform_name() -> &'static str {
        "macos"
    }

    pub fn supported() -> bool {
        true
    }

    pub fn check_item(id: &str) -> SystemPermissionStatus {
        match id {
            "accessibility" => bool_status(unsafe { AXIsProcessTrusted() }),
            "screen_recording" => bool_status(unsafe { CGPreflightScreenCaptureAccess() }),
            "input_monitoring" => bool_status(unsafe { CGPreflightListenEventAccess() }),
            "camera" => av_media_status("vide"),
            "microphone" => av_media_status("soun"),
            "location" => location_status(),
            "contacts" => auth_status_with_entity(c"CNContactStore", 0),
            "calendar" => auth_status_with_entity(c"EKEventStore", 0),
            "reminders" => auth_status_with_entity(c"EKEventStore", 1),
            "photos" => photos_status(),
            "bluetooth" => objc_auth_status(c"CBCentralManager", "authorization"),
            "speech_recognition" => speech_status(),
            "notifications" => notification_status(),
            "full_disk_access" => full_disk_access_status(),
            "desktop_folder" => folder_status("Desktop"),
            "documents_folder" => folder_status("Documents"),
            "downloads_folder" => folder_status("Downloads"),
            "homekit" => SystemPermissionStatus::NotUsed,
            "automation_system_events"
            | "automation_messages"
            | "app_management"
            | "developer_tools"
            | "system_audio_capture"
            | "removable_volumes"
            | "network_volumes"
            | "media_library"
            | "focus_status"
            | "local_network" => SystemPermissionStatus::ManualCheck,
            _ => SystemPermissionStatus::NotApplicable,
        }
    }

    pub fn request_item(def: PermissionDef) -> SystemPermissionStatus {
        match def.id {
            "screen_recording" => {
                let ok = unsafe { CGRequestScreenCaptureAccess() };
                if !ok {
                    open_settings_pane(def.settings_pane);
                }
                bool_status(ok)
            }
            "input_monitoring" => {
                let ok = unsafe { CGRequestListenEventAccess() };
                if !ok {
                    open_settings_pane(def.settings_pane);
                }
                bool_status(ok)
            }
            "camera" => request_av_media(def, "vide"),
            "microphone" => request_av_media(def, "soun"),
            "location" => request_location(def),
            "contacts" => request_contacts(def),
            "calendar" => request_eventkit(def, 0),
            "reminders" => request_eventkit(def, 1),
            "photos" => request_photos(def),
            "bluetooth" => request_bluetooth(def),
            "speech_recognition" => request_speech(def),
            "automation_system_events" => {
                trigger_automation_probe("System Events");
                open_settings_pane(def.settings_pane);
                check_item(def.id)
            }
            "automation_messages" => {
                trigger_automation_probe("Messages");
                open_settings_pane(def.settings_pane);
                check_item(def.id)
            }
            _ => {
                open_settings_pane(def.settings_pane);
                check_item(def.id)
            }
        }
    }

    fn bool_status(value: bool) -> SystemPermissionStatus {
        if value {
            SystemPermissionStatus::Granted
        } else {
            SystemPermissionStatus::NotGranted
        }
    }

    fn map_standard_auth_status(status: isize) -> SystemPermissionStatus {
        match status {
            0 => SystemPermissionStatus::NotDetermined,
            1 => SystemPermissionStatus::Restricted,
            2 => SystemPermissionStatus::NotGranted,
            // Some frameworks use 4 for "limited" or "when in use"; both
            // mean the app has usable access for this permissions overview.
            3 | 4 => SystemPermissionStatus::Granted,
            _ => SystemPermissionStatus::ManualCheck,
        }
    }

    fn map_speech_auth_status(status: isize) -> SystemPermissionStatus {
        match status {
            0 => SystemPermissionStatus::NotDetermined,
            1 => SystemPermissionStatus::NotGranted,
            2 => SystemPermissionStatus::Restricted,
            3 => SystemPermissionStatus::Granted,
            _ => SystemPermissionStatus::ManualCheck,
        }
    }

    fn map_notification_auth_status(status: isize) -> SystemPermissionStatus {
        match status {
            0 => SystemPermissionStatus::NotDetermined,
            1 => SystemPermissionStatus::NotGranted,
            2..=4 => SystemPermissionStatus::Granted,
            _ => SystemPermissionStatus::ManualCheck,
        }
    }

    fn wait_for_prompt(
        id: &str,
        receiver: mpsc::Receiver<SystemPermissionStatus>,
    ) -> SystemPermissionStatus {
        receiver
            .recv_timeout(Duration::from_secs(60))
            .unwrap_or_else(|_| check_item(id))
    }

    fn requestable_status_or_open(
        def: PermissionDef,
        status: SystemPermissionStatus,
    ) -> Option<SystemPermissionStatus> {
        if status == SystemPermissionStatus::NotDetermined {
            None
        } else {
            open_settings_pane(def.settings_pane);
            Some(status)
        }
    }

    fn objc_class(name: &'static CStr) -> Option<&'static AnyClass> {
        AnyClass::get(name)
    }

    fn objc_auth_status(class_name: &'static CStr, selector: &str) -> SystemPermissionStatus {
        let Some(cls) = objc_class(class_name) else {
            return SystemPermissionStatus::NotApplicable;
        };
        let status: isize = unsafe {
            match selector {
                "authorization" => msg_send![cls, authorization],
                "authorizationStatus" => msg_send![cls, authorizationStatus],
                _ => return SystemPermissionStatus::NotApplicable,
            }
        };
        map_standard_auth_status(status)
    }

    fn auth_status_with_entity(
        class_name: &'static CStr,
        entity_type: isize,
    ) -> SystemPermissionStatus {
        let Some(cls) = objc_class(class_name) else {
            return SystemPermissionStatus::NotApplicable;
        };
        let status: isize =
            unsafe { msg_send![cls, authorizationStatusForEntityType: entity_type] };
        map_standard_auth_status(status)
    }

    fn av_media_status(raw_media_type: &str) -> SystemPermissionStatus {
        let Some(cls) = objc_class(c"AVCaptureDevice") else {
            return SystemPermissionStatus::NotApplicable;
        };
        let media_type = NSString::from_str(raw_media_type);
        let status: isize =
            unsafe { msg_send![cls, authorizationStatusForMediaType: &*media_type] };
        map_standard_auth_status(status)
    }

    fn request_av_media(def: PermissionDef, raw_media_type: &str) -> SystemPermissionStatus {
        let status = av_media_status(raw_media_type);
        if let Some(status) = requestable_status_or_open(def, status) {
            return status;
        }

        let Some(cls) = objc_class(c"AVCaptureDevice") else {
            return SystemPermissionStatus::NotApplicable;
        };
        let media_type = NSString::from_str(raw_media_type);
        let (sender, receiver) = mpsc::channel();
        let block = RcBlock::new(move |granted: Bool| {
            let _ = sender.send(bool_status(granted.as_bool()));
        });

        unsafe {
            let _: () = msg_send![
                cls,
                requestAccessForMediaType: &*media_type,
                completionHandler: &*block
            ];
        }

        wait_for_prompt(def.id, receiver)
    }

    fn location_status() -> SystemPermissionStatus {
        let Some(cls) = objc_class(c"CLLocationManager") else {
            return SystemPermissionStatus::NotApplicable;
        };
        let status: isize = unsafe { msg_send![cls, authorizationStatus] };
        match status {
            0 => SystemPermissionStatus::NotDetermined,
            1 => SystemPermissionStatus::Restricted,
            2 => SystemPermissionStatus::NotGranted,
            3 | 4 => SystemPermissionStatus::Granted,
            _ => SystemPermissionStatus::ManualCheck,
        }
    }

    fn request_location(def: PermissionDef) -> SystemPermissionStatus {
        let status = location_status();
        if let Some(status) = requestable_status_or_open(def, status) {
            return status;
        }

        let Some(cls) = objc_class(c"CLLocationManager") else {
            return SystemPermissionStatus::NotApplicable;
        };
        let manager: Retained<AnyObject> = unsafe { msg_send![cls, new] };
        unsafe {
            let _: () = msg_send![&*manager, requestWhenInUseAuthorization];
        }
        std::thread::sleep(Duration::from_millis(500));
        check_item(def.id)
    }

    fn request_contacts(def: PermissionDef) -> SystemPermissionStatus {
        let status = auth_status_with_entity(c"CNContactStore", 0);
        if let Some(status) = requestable_status_or_open(def, status) {
            return status;
        }

        let Some(cls) = objc_class(c"CNContactStore") else {
            return SystemPermissionStatus::NotApplicable;
        };
        let store: Retained<AnyObject> = unsafe { msg_send![cls, new] };
        let (sender, receiver) = mpsc::channel();
        let block = RcBlock::new(move |granted: Bool, _error: *mut AnyObject| {
            let _ = sender.send(bool_status(granted.as_bool()));
        });

        unsafe {
            let _: () = msg_send![
                &*store,
                requestAccessForEntityType: 0isize,
                completionHandler: &*block
            ];
        }

        wait_for_prompt(def.id, receiver)
    }

    fn request_eventkit(def: PermissionDef, entity_type: isize) -> SystemPermissionStatus {
        let status = auth_status_with_entity(c"EKEventStore", entity_type);
        if let Some(status) = requestable_status_or_open(def, status) {
            return status;
        }

        let Some(cls) = objc_class(c"EKEventStore") else {
            return SystemPermissionStatus::NotApplicable;
        };
        let store: Retained<AnyObject> = unsafe { msg_send![cls, new] };
        let (sender, receiver) = mpsc::channel();
        let block = RcBlock::new(move |granted: Bool, _error: *mut AnyObject| {
            let _ = sender.send(bool_status(granted.as_bool()));
        });

        unsafe {
            let _: () = msg_send![
                &*store,
                requestAccessToEntityType: entity_type,
                completion: &*block
            ];
        }

        wait_for_prompt(def.id, receiver)
    }

    fn photos_status() -> SystemPermissionStatus {
        let Some(cls) = objc_class(c"PHPhotoLibrary") else {
            return SystemPermissionStatus::NotApplicable;
        };
        let status: isize = unsafe { msg_send![cls, authorizationStatus] };
        map_standard_auth_status(status)
    }

    fn request_photos(def: PermissionDef) -> SystemPermissionStatus {
        let status = photos_status();
        if let Some(status) = requestable_status_or_open(def, status) {
            return status;
        }

        let Some(cls) = objc_class(c"PHPhotoLibrary") else {
            return SystemPermissionStatus::NotApplicable;
        };
        let (sender, receiver) = mpsc::channel();
        let block = RcBlock::new(move |status: isize| {
            let _ = sender.send(map_standard_auth_status(status));
        });

        unsafe {
            let _: () = msg_send![cls, requestAuthorization: &*block];
        }

        wait_for_prompt(def.id, receiver)
    }

    fn speech_status() -> SystemPermissionStatus {
        let Some(cls) = objc_class(c"SFSpeechRecognizer") else {
            return SystemPermissionStatus::NotApplicable;
        };
        let status: isize = unsafe { msg_send![cls, authorizationStatus] };
        map_speech_auth_status(status)
    }

    fn request_speech(def: PermissionDef) -> SystemPermissionStatus {
        let status = speech_status();
        if let Some(status) = requestable_status_or_open(def, status) {
            return status;
        }

        let Some(cls) = objc_class(c"SFSpeechRecognizer") else {
            return SystemPermissionStatus::NotApplicable;
        };
        let (sender, receiver) = mpsc::channel();
        let block = RcBlock::new(move |status: isize| {
            let _ = sender.send(map_speech_auth_status(status));
        });

        unsafe {
            let _: () = msg_send![cls, requestAuthorization: &*block];
        }

        wait_for_prompt(def.id, receiver)
    }

    fn request_bluetooth(def: PermissionDef) -> SystemPermissionStatus {
        let status = objc_auth_status(c"CBCentralManager", "authorization");
        if let Some(status) = requestable_status_or_open(def, status) {
            return status;
        }

        let Some(cls) = objc_class(c"CBCentralManager") else {
            return SystemPermissionStatus::NotApplicable;
        };
        let manager: *mut AnyObject = unsafe { msg_send![cls, alloc] };
        let manager: *mut AnyObject = unsafe {
            msg_send![
                manager,
                initWithDelegate: ptr::null_mut::<AnyObject>(),
                queue: ptr::null_mut::<AnyObject>(),
                options: ptr::null_mut::<AnyObject>()
            ]
        };
        std::thread::sleep(Duration::from_millis(500));
        if !manager.is_null() {
            unsafe {
                let _: () = msg_send![manager, release];
            }
        }
        check_item(def.id)
    }

    fn notification_status() -> SystemPermissionStatus {
        let Some(cls) = objc_class(c"UNUserNotificationCenter") else {
            return SystemPermissionStatus::NotApplicable;
        };
        let center: Retained<AnyObject> = unsafe { msg_send![cls, currentNotificationCenter] };
        let (sender, receiver) = mpsc::channel();
        let block = RcBlock::new(move |settings: *mut AnyObject| {
            if settings.is_null() {
                let _ = sender.send(SystemPermissionStatus::ManualCheck);
                return;
            }
            let status: isize = unsafe { msg_send![settings, authorizationStatus] };
            let _ = sender.send(map_notification_auth_status(status));
        });

        unsafe {
            let _: () = msg_send![&*center, getNotificationSettingsWithCompletionHandler: &*block];
        }

        receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap_or(SystemPermissionStatus::ManualCheck)
    }

    fn full_disk_access_status() -> SystemPermissionStatus {
        let Some(home) = dirs::home_dir() else {
            return SystemPermissionStatus::ManualCheck;
        };
        let probes = [
            home.join("Library/Safari/Bookmarks.plist"),
            home.join("Library/Messages/chat.db"),
        ];
        if probes.iter().any(|path| std::fs::metadata(path).is_ok()) {
            SystemPermissionStatus::Granted
        } else {
            SystemPermissionStatus::ManualCheck
        }
    }

    fn folder_status(folder: &str) -> SystemPermissionStatus {
        let Some(home) = dirs::home_dir() else {
            return SystemPermissionStatus::ManualCheck;
        };
        let path = home.join(folder);
        if can_read_dir(&path) {
            SystemPermissionStatus::Granted
        } else {
            SystemPermissionStatus::ManualCheck
        }
    }

    fn can_read_dir(path: &Path) -> bool {
        std::fs::read_dir(path).is_ok()
    }

    fn trigger_automation_probe(target: &str) {
        let script = format!("tell application \"{}\" to return name", target);
        let _ = Command::new("osascript")
            .args(["-e", &script])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .and_then(|mut child| child.wait());
    }

    fn open_settings_pane(pane: Option<&str>) {
        let Some(pane) = pane else {
            return;
        };
        let url = match pane {
            "Notifications" => {
                "x-apple.systempreferences:com.apple.preference.notifications".to_string()
            }
            pane => format!(
                "x-apple.systempreferences:com.apple.preference.security?{}",
                pane
            ),
        };
        let _ = Command::new("open").arg(url).spawn();
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;

    pub fn platform_name() -> &'static str {
        "windows"
    }

    pub fn supported() -> bool {
        false
    }

    pub fn check_item(_id: &str) -> SystemPermissionStatus {
        SystemPermissionStatus::NotApplicable
    }

    pub fn request_item(_def: PermissionDef) -> SystemPermissionStatus {
        SystemPermissionStatus::NotApplicable
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;

    pub fn platform_name() -> &'static str {
        "linux"
    }

    pub fn supported() -> bool {
        false
    }

    pub fn check_item(_id: &str) -> SystemPermissionStatus {
        SystemPermissionStatus::NotApplicable
    }

    pub fn request_item(_def: PermissionDef) -> SystemPermissionStatus {
        SystemPermissionStatus::NotApplicable
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod platform {
    use super::*;

    pub fn platform_name() -> &'static str {
        "unknown"
    }

    pub fn supported() -> bool {
        false
    }

    pub fn check_item(_id: &str) -> SystemPermissionStatus {
        SystemPermissionStatus::NotApplicable
    }

    pub fn request_item(_def: PermissionDef) -> SystemPermissionStatus {
        SystemPermissionStatus::NotApplicable
    }
}

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
        let supported = platform::supported();
        let items = if supported {
            PERMISSION_DEFS
                .iter()
                .map(|def| def.item(platform::check_item(def.id)))
                .collect()
        } else {
            Vec::new()
        };
        SystemPermissionsResponse {
            platform: platform::platform_name().to_string(),
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
        let status = if platform::supported() {
            platform::request_item(def)
        } else {
            SystemPermissionStatus::NotApplicable
        };
        def.item(status)
    })
    .await
}

fn unsupported_response() -> SystemPermissionsResponse {
    SystemPermissionsResponse {
        platform: platform::platform_name().to_string(),
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
        SystemPermissionStatus::NotGranted
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
                }
                .item(SystemPermissionStatus::Granted),
                PermissionDef {
                    id: "desktop_folder",
                    group: SystemPermissionGroup::FileAccess,
                    request_mode: SystemPermissionRequestMode::OpenSettings,
                    settings_pane: None,
                    usage: "",
                    note: None,
                }
                .item(SystemPermissionStatus::Granted),
                PermissionDef {
                    id: "documents_folder",
                    group: SystemPermissionGroup::FileAccess,
                    request_mode: SystemPermissionRequestMode::OpenSettings,
                    settings_pane: None,
                    usage: "",
                    note: None,
                }
                .item(SystemPermissionStatus::Granted),
                PermissionDef {
                    id: "downloads_folder",
                    group: SystemPermissionGroup::FileAccess,
                    request_mode: SystemPermissionRequestMode::OpenSettings,
                    settings_pane: None,
                    usage: "",
                    note: None,
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
