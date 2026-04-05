//! macOS system permission checking & requesting.
//!
//! Covers 15 permissions for "computer control".
//! All checks run in parallel via `tokio::spawn_blocking` with per-check timeouts
//! to avoid blocking the main thread.

use serde::Serialize;
use std::time::Duration;

/// Per-check timeout for subprocess-based checks (osascript, etc.)
const CHECK_TIMEOUT: Duration = Duration::from_secs(3);

// ── Public data types ────────────────────────────────────────────

/// Permission check result: "granted" | "not_granted" | "unknown"
/// "unknown" = cannot be programmatically detected, user must verify in System Settings.
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

#[derive(Debug, Clone, Serialize, Default)]
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

// ── Platform-specific implementation ─────────────────────────────

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::process::Command;

    // ── Framework bindings (C ABI) ──

    extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
    }

    // ── Helpers ──

    fn bool_to_state(b: bool) -> PermState {
        if b {
            granted()
        } else {
            not_granted()
        }
    }

    fn run_osascript(args: &[&str]) -> Option<std::process::Output> {
        Command::new("osascript")
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .ok()?
            .wait_with_output()
            .ok()
    }

    fn jxa(script: &str) -> Option<String> {
        run_osascript(&["-l", "JavaScript", "-e", script])
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    }

    /// Check a framework authorization status via JXA. Returns granted if status == 3.
    fn jxa_auth_status(script: &str) -> PermState {
        bool_to_state(jxa(script).map_or(false, |s| s.trim() == "3"))
    }

    pub fn open_privacy_pane(pane: &str) {
        let url = format!(
            "x-apple.systempreferences:com.apple.preference.security?{}",
            pane
        );
        let _ = Command::new("open").arg(&url).spawn();
    }

    // ── 1. Accessibility ── (instant, C API)

    pub fn check_accessibility() -> PermState {
        bool_to_state(unsafe { AXIsProcessTrusted() })
    }

    pub fn request_accessibility() -> PermState {
        open_privacy_pane("Privacy_Accessibility");
        check_accessibility()
    }

    // ── 2. Screen Recording ── (instant, C API)

    pub fn check_screen_recording() -> PermState {
        bool_to_state(unsafe { CGPreflightScreenCaptureAccess() })
    }

    pub fn request_screen_recording() -> PermState {
        let ok = unsafe { CGRequestScreenCaptureAccess() };
        if !ok {
            open_privacy_pane("Privacy_ScreenCapture");
        }
        bool_to_state(ok)
    }

    // ── 3. Automation ── (osascript)

    pub fn check_automation() -> PermState {
        bool_to_state(
            run_osascript(&["-e", "tell application \"System Events\" to return name"])
                .map_or(false, |o| o.status.success()),
        )
    }

    pub fn request_automation() -> PermState {
        let _ = run_osascript(&["-e", "tell application \"System Events\" to return name"]);
        open_privacy_pane("Privacy_Automation");
        check_automation()
    }

    // ── 4. App Management ──
    // Cannot be reliably detected via filesystem ops (TCC != Unix perms).

    pub fn check_app_management() -> PermState {
        unknown()
    }

    pub fn request_app_management() -> PermState {
        open_privacy_pane("Privacy_AppBundles");
        unknown()
    }

    // ── 5. Full Disk Access ── (filesystem heuristic)

    pub fn check_full_disk_access() -> PermState {
        bool_to_state(dirs::home_dir().map_or(false, |h| {
            std::fs::metadata(h.join("Library/Safari/Bookmarks.plist")).is_ok()
        }))
    }

    pub fn request_full_disk_access() -> PermState {
        open_privacy_pane("Privacy_AllFiles");
        check_full_disk_access()
    }

    // ── 6. Location Services ── (JXA)

    pub fn check_location() -> PermState {
        bool_to_state(
            jxa("ObjC.import('CoreLocation'); \
                 var s = $.CLLocationManager.authorizationStatus; \
                 s === 3 || s === 4")
            .map_or(false, |s| s == "true"),
        )
    }

    pub fn request_location() -> PermState {
        open_privacy_pane("Privacy_LocationServices");
        check_location()
    }

    // ── 7. Contacts ── (JXA)

    pub fn check_contacts() -> PermState {
        jxa_auth_status(
            "ObjC.import('Contacts'); \
             $.CNContactStore.authorizationStatusForEntityType(0)",
        )
    }

    pub fn request_contacts() -> PermState {
        open_privacy_pane("Privacy_Contacts");
        check_contacts()
    }

    // ── 8. Calendar ── (JXA)

    pub fn check_calendar() -> PermState {
        jxa_auth_status(
            "ObjC.import('EventKit'); \
             $.EKEventStore.authorizationStatusForEntityType(0)",
        )
    }

    pub fn request_calendar() -> PermState {
        open_privacy_pane("Privacy_Calendars");
        check_calendar()
    }

    // ── 9. Reminders ── (JXA)

    pub fn check_reminders() -> PermState {
        jxa_auth_status(
            "ObjC.import('EventKit'); \
             $.EKEventStore.authorizationStatusForEntityType(1)",
        )
    }

    pub fn request_reminders() -> PermState {
        open_privacy_pane("Privacy_Reminders");
        check_reminders()
    }

    // ── 10. Photos ── (JXA)

    pub fn check_photos() -> PermState {
        jxa_auth_status(
            "ObjC.import('Photos'); \
             $.PHPhotoLibrary.authorizationStatus",
        )
    }

    pub fn request_photos() -> PermState {
        open_privacy_pane("Privacy_Photos");
        check_photos()
    }

    // ── 11. Camera ── (JXA)

    pub fn check_camera() -> PermState {
        jxa_auth_status(
            "ObjC.import('AVFoundation'); \
             $.AVCaptureDevice.authorizationStatusForMediaType('vide')",
        )
    }

    pub fn request_camera() -> PermState {
        open_privacy_pane("Privacy_Camera");
        check_camera()
    }

    // ── 12. Microphone ── (JXA)

    pub fn check_microphone() -> PermState {
        jxa_auth_status(
            "ObjC.import('AVFoundation'); \
             $.AVCaptureDevice.authorizationStatusForMediaType('soun')",
        )
    }

    pub fn request_microphone() -> PermState {
        open_privacy_pane("Privacy_Microphone");
        check_microphone()
    }

    // ── 13. Local Network ──
    // Cannot be reliably detected programmatically on macOS.

    pub fn check_local_network() -> PermState {
        unknown()
    }

    pub fn request_local_network() -> PermState {
        open_privacy_pane("Privacy_LocalNetwork");
        unknown()
    }

    // ── 14. Bluetooth ── (JXA)

    pub fn check_bluetooth() -> PermState {
        bool_to_state(
            jxa("ObjC.import('CoreBluetooth'); \
                 var auth = $.CBCentralManager.authorization; \
                 auth === 3")
            .map_or(false, |s| s == "true"),
        )
    }

    pub fn request_bluetooth() -> PermState {
        open_privacy_pane("Privacy_Bluetooth");
        check_bluetooth()
    }

    // ── 15. Files & Folders ── (filesystem, instant)

    pub fn check_files_and_folders() -> PermState {
        bool_to_state(dirs::home_dir().map_or(false, |home| {
            std::fs::read_dir(home.join("Desktop")).is_ok()
                && std::fs::read_dir(home.join("Documents")).is_ok()
                && std::fs::read_dir(home.join("Downloads")).is_ok()
        }))
    }

    pub fn request_files_and_folders() -> PermState {
        open_privacy_pane("Privacy_FilesAndFolders");
        check_files_and_folders()
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::*;

    pub fn check_accessibility() -> PermState {
        granted()
    }
    pub fn request_accessibility() -> PermState {
        granted()
    }
    pub fn check_screen_recording() -> PermState {
        granted()
    }
    pub fn request_screen_recording() -> PermState {
        granted()
    }
    pub fn check_automation() -> PermState {
        granted()
    }
    pub fn request_automation() -> PermState {
        granted()
    }
    pub fn check_app_management() -> PermState {
        granted()
    }
    pub fn request_app_management() -> PermState {
        granted()
    }
    pub fn check_full_disk_access() -> PermState {
        granted()
    }
    pub fn request_full_disk_access() -> PermState {
        granted()
    }
    pub fn check_location() -> PermState {
        granted()
    }
    pub fn request_location() -> PermState {
        granted()
    }
    pub fn check_contacts() -> PermState {
        granted()
    }
    pub fn request_contacts() -> PermState {
        granted()
    }
    pub fn check_calendar() -> PermState {
        granted()
    }
    pub fn request_calendar() -> PermState {
        granted()
    }
    pub fn check_reminders() -> PermState {
        granted()
    }
    pub fn request_reminders() -> PermState {
        granted()
    }
    pub fn check_photos() -> PermState {
        granted()
    }
    pub fn request_photos() -> PermState {
        granted()
    }
    pub fn check_camera() -> PermState {
        granted()
    }
    pub fn request_camera() -> PermState {
        granted()
    }
    pub fn check_microphone() -> PermState {
        granted()
    }
    pub fn request_microphone() -> PermState {
        granted()
    }
    pub fn check_local_network() -> PermState {
        granted()
    }
    pub fn request_local_network() -> PermState {
        granted()
    }
    pub fn check_bluetooth() -> PermState {
        granted()
    }
    pub fn request_bluetooth() -> PermState {
        granted()
    }
    pub fn check_files_and_folders() -> PermState {
        granted()
    }
    pub fn request_files_and_folders() -> PermState {
        granted()
    }

    pub fn open_privacy_pane(_: &str) {}
}

// ── Async helpers ────────────────────────────────────────────────

/// Run a blocking check function with a timeout. Returns "not_granted" on timeout.
async fn check_with_timeout<F: FnOnce() -> PermState + Send + 'static>(f: F) -> PermState {
    match tokio::time::timeout(CHECK_TIMEOUT, tokio::task::spawn_blocking(f)).await {
        Ok(Ok(result)) => result,
        _ => not_granted(), // timeout or panic → treat as not granted
    }
}

// ── Tauri commands (all async) ───────────────────────────────────

pub async fn check_all_permissions() -> AllPermissions {
    // Run all 15 checks in parallel with timeouts
    let (
        accessibility,
        screen_recording,
        automation,
        app_management,
        full_disk_access,
        location,
        contacts,
        calendar,
        reminders,
        photos,
        camera,
        microphone,
        local_network,
        bluetooth,
        files_and_folders,
    ) = tokio::join!(
        check_with_timeout(platform::check_accessibility),
        check_with_timeout(platform::check_screen_recording),
        check_with_timeout(platform::check_automation),
        check_with_timeout(platform::check_app_management),
        check_with_timeout(platform::check_full_disk_access),
        check_with_timeout(platform::check_location),
        check_with_timeout(platform::check_contacts),
        check_with_timeout(platform::check_calendar),
        check_with_timeout(platform::check_reminders),
        check_with_timeout(platform::check_photos),
        check_with_timeout(platform::check_camera),
        check_with_timeout(platform::check_microphone),
        check_with_timeout(platform::check_local_network),
        check_with_timeout(platform::check_bluetooth),
        check_with_timeout(platform::check_files_and_folders),
    );

    let r = AllPermissions {
        accessibility,
        screen_recording,
        automation,
        app_management,
        full_disk_access,
        location,
        contacts,
        calendar,
        reminders,
        photos,
        camera,
        microphone,
        local_network,
        bluetooth,
        files_and_folders,
    };

    crate::app_info!(
        "permissions",
        "check_all",
        "a11y={} screen={} auto={} appmgmt={} fda={} loc={} contacts={} cal={} remind={} photos={} cam={} mic={} net={} bt={} files={}",
        r.accessibility, r.screen_recording, r.automation, r.app_management,
        r.full_disk_access, r.location, r.contacts, r.calendar, r.reminders,
        r.photos, r.camera, r.microphone, r.local_network, r.bluetooth,
        r.files_and_folders
    );
    r
}

pub async fn check_permission(id: String) -> PermissionStatus {
    let id2 = id.clone();
    let status = check_with_timeout(move || dispatch_check(&id2)).await;
    PermissionStatus { id, status }
}

pub async fn request_permission(id: String) -> PermissionStatus {
    crate::app_info!("permissions", "request", "Requesting: {}", id);
    let id2 = id.clone();
    let status = check_with_timeout(move || dispatch_request(&id2)).await;
    crate::app_info!("permissions", "request", "{} → {}", id, status);
    PermissionStatus { id, status }
}

fn dispatch_check(id: &str) -> PermState {
    match id {
        "accessibility" => platform::check_accessibility(),
        "screen_recording" => platform::check_screen_recording(),
        "automation" => platform::check_automation(),
        "app_management" => platform::check_app_management(),
        "full_disk_access" => platform::check_full_disk_access(),
        "location" => platform::check_location(),
        "contacts" => platform::check_contacts(),
        "calendar" => platform::check_calendar(),
        "reminders" => platform::check_reminders(),
        "photos" => platform::check_photos(),
        "camera" => platform::check_camera(),
        "microphone" => platform::check_microphone(),
        "local_network" => platform::check_local_network(),
        "bluetooth" => platform::check_bluetooth(),
        "files_and_folders" => platform::check_files_and_folders(),
        _ => not_granted(),
    }
}

fn dispatch_request(id: &str) -> PermState {
    match id {
        "accessibility" => platform::request_accessibility(),
        "screen_recording" => platform::request_screen_recording(),
        "automation" => platform::request_automation(),
        "app_management" => platform::request_app_management(),
        "full_disk_access" => platform::request_full_disk_access(),
        "location" => platform::request_location(),
        "contacts" => platform::request_contacts(),
        "calendar" => platform::request_calendar(),
        "reminders" => platform::request_reminders(),
        "photos" => platform::request_photos(),
        "camera" => platform::request_camera(),
        "microphone" => platform::request_microphone(),
        "local_network" => platform::request_local_network(),
        "bluetooth" => platform::request_bluetooth(),
        "files_and_folders" => platform::request_files_and_folders(),
        _ => not_granted(),
    }
}
