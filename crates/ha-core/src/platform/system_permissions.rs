//! Platform-specific system permission checks and request prompts.
//!
//! This module is intentionally called from `crate::permissions`; it owns the
//! OS-native implementation while the public catalog / response shaping stays
//! in the permissions domain module.

use crate::permissions::{PermissionDef, SystemPermissionStatus};

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::ffi::CStr;
    use std::path::Path;
    use std::process::Command;
    use std::ptr;
    use std::sync::mpsc;
    use std::time::Duration;

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

    fn running_from_app_bundle() -> bool {
        std::env::current_exe()
            .ok()
            .is_some_and(|exe| path_is_in_app_bundle(&exe))
    }

    fn path_is_in_app_bundle(path: &Path) -> bool {
        path.ancestors().any(|ancestor| {
            ancestor
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
        })
    }

    fn notification_status() -> SystemPermissionStatus {
        // `UNUserNotificationCenter.currentNotificationCenter()` raises an
        // Objective-C NSException when the process is a bare debug binary
        // (`target/debug/hope-agent`) instead of a real `.app` bundle. Rust
        // cannot catch that exception, so skip the native query in unbundled
        // dev/CLI contexts and let the UI present this as a manual check.
        if !running_from_app_bundle() {
            return SystemPermissionStatus::ManualCheck;
        }

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

    #[cfg(test)]
    mod tests {
        use super::path_is_in_app_bundle;
        use std::path::Path;

        #[test]
        fn detects_executable_inside_app_bundle() {
            assert!(path_is_in_app_bundle(Path::new(
                "/Applications/Hope Agent.app/Contents/MacOS/hope-agent"
            )));
            assert!(path_is_in_app_bundle(Path::new(
                "/tmp/target/debug/bundle/macos/Hope Agent.app/Contents/MacOS/hope-agent"
            )));
        }

        #[test]
        fn rejects_bare_debug_executable() {
            assert!(!path_is_in_app_bundle(Path::new(
                "/Users/me/Codes/hope-agent/target/debug/hope-agent"
            )));
        }
    }
}

#[cfg(target_os = "windows")]
mod imp {
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
mod imp {
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
mod imp {
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

pub(crate) fn platform_name() -> &'static str {
    imp::platform_name()
}

pub(crate) fn supported() -> bool {
    imp::supported()
}

pub(crate) fn check_item(id: &str) -> SystemPermissionStatus {
    imp::check_item(id)
}

pub(crate) fn request_item(def: PermissionDef) -> SystemPermissionStatus {
    imp::request_item(def)
}
