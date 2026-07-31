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
    use std::process::{Command, Stdio};
    use std::ptr;
    use std::sync::{mpsc, Mutex};
    use std::time::{Duration, Instant};

    use block2::RcBlock;
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2_foundation::NSString;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: *const AnyObject) -> bool;
        static kAXTrustedCheckOptionPrompt: *const AnyObject;
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
            "screen_recording" => screen_recording_status(false),
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
            "system_audio_capture" => SystemPermissionStatus::NotUsed,
            "homekit" => SystemPermissionStatus::NotUsed,
            "automation_system_events"
            | "automation_messages"
            | "app_management"
            | "developer_tools"
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
            "accessibility" => {
                let ok = request_accessibility_trust();
                if !ok {
                    open_settings_pane(def.settings_pane);
                }
                bool_status(ok)
            }
            "screen_recording" => {
                if unsafe { CGRequestScreenCaptureAccess() } {
                    return SystemPermissionStatus::Granted;
                }
                // Bypass the negative-probe debounce: the user may have just
                // flipped the toggle in System Settings, which this running
                // process cannot observe on its own.
                let status = screen_recording_status(true);
                // Open the pane even for pending-restart: the documented
                // remediation for a stale TCC entry is to remove and re-add
                // it there, so the jump must stay available.
                open_settings_pane(def.settings_pane);
                status
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
            "notifications" => request_notifications(def),
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

    /// Ask for Accessibility trust with the system consent prompt enabled.
    ///
    /// Unlike the plain `AXIsProcessTrusted` preflight, the prompting variant
    /// registers this app in System Settings → Privacy & Security →
    /// Accessibility. Without that registration the pane simply has no row
    /// for the app and the user has nothing to toggle.
    ///
    /// Note the asymmetry with the other NativePrompt permissions: this call
    /// returns the CURRENT (still-false) trust state synchronously while the
    /// consent dialog it posts is answered asynchronously, and macOS shows
    /// that dialog at most once per app. So the caller also opens the
    /// settings pane on `false` — a deliberate double surface, because the
    /// alternative (trusting the synchronous `false` and doing nothing) is
    /// the "clicked Grant, nothing happened" dead end this fix exists to
    /// remove.
    fn request_accessibility_trust() -> bool {
        let (Some(dict_cls), Some(number_cls)) =
            (objc_class(c"NSDictionary"), objc_class(c"NSNumber"))
        else {
            return unsafe { AXIsProcessTrusted() };
        };
        // Runs on a tokio blocking-pool thread, which has no autorelease
        // pool of its own; the autoreleased dictionary would leak (and log
        // "autoreleased with no pool in place") without this.
        objc2::rc::autoreleasepool(|_| unsafe {
            let prompt_key = kAXTrustedCheckOptionPrompt;
            if prompt_key.is_null() {
                return AXIsProcessTrusted();
            }
            let yes: Retained<AnyObject> = msg_send![number_cls, numberWithBool: Bool::YES];
            let options: Retained<AnyObject> =
                msg_send![dict_cls, dictionaryWithObject: &*yes, forKey: prompt_key];
            AXIsProcessTrustedWithOptions(Retained::as_ptr(&options))
        })
    }

    /// Screen Recording capability is fixed on this process's window-server
    /// connection at launch: a grant made while the app is running keeps
    /// preflighting `false` until relaunch, while a freshly launched process
    /// sees the live TCC state. When the in-process preflight is negative,
    /// probe from a short-lived child (same executable → same TCC identity
    /// through the responsible process) to distinguish "granted, needs an
    /// app restart" from "really not granted".
    fn screen_recording_status(bypass_probe_debounce: bool) -> SystemPermissionStatus {
        if unsafe { CGPreflightScreenCaptureAccess() } {
            return SystemPermissionStatus::Granted;
        }
        if screen_probe_pending_restart(bypass_probe_debounce) {
            SystemPermissionStatus::GrantedPendingRestart
        } else {
            SystemPermissionStatus::NotGranted
        }
    }

    /// How long a negative probe result suppresses re-spawning. `check_item`
    /// runs on every panel fetch (window-focus re-checks included) and every
    /// mac_control preflight — a stuck-at-not-granted state must not spawn a
    /// child process per call.
    const PROBE_RETRY_TTL: Duration = Duration::from_secs(5);
    /// Must stay comfortably under the catalog-wide `CHECK_TIMEOUT` in
    /// `permissions.rs` together with the other slow items (the notifications
    /// query alone can take 2s).
    const PROBE_WAIT: Duration = Duration::from_millis(1500);

    /// Screen-recording probe bookkeeping. Deliberately NOT
    /// `crate::ttl_cache::TtlCache`: this is not a keyed TTL map but
    /// single-permission state needing (a) single-flight — the lock is held
    /// ACROSS the child probe so concurrent catalog passes share one spawn —
    /// and (b) a sticky positive: once a fresh process has observed the
    /// grant, that fact cannot regress until relaunch, so it is kept for the
    /// process lifetime and never re-probed (nor overwritten by a later
    /// transient probe failure). Only the negative result is retried, at
    /// most once per `PROBE_RETRY_TTL`.
    struct ScreenProbeState {
        pending_restart_confirmed: bool,
        last_negative_at: Option<Instant>,
    }

    static SCREEN_PROBE: Mutex<ScreenProbeState> = Mutex::new(ScreenProbeState {
        pending_restart_confirmed: false,
        last_negative_at: None,
    });

    fn screen_probe_pending_restart(bypass_debounce: bool) -> bool {
        let Ok(mut state) = SCREEN_PROBE.lock() else {
            return false;
        };
        if state.pending_restart_confirmed {
            return true;
        }
        if !bypass_debounce {
            if let Some(at) = state.last_negative_at {
                if at.elapsed() < PROBE_RETRY_TTL {
                    return false;
                }
            }
        }
        let result = fresh_process_probe("screen_recording");
        crate::app_info!(
            "permissions",
            "tcc_probe",
            "screen_recording fresh-process probe → {:?}",
            result
        );
        if result == Some(true) {
            state.pending_restart_confirmed = true;
            true
        } else {
            state.last_negative_at = Some(Instant::now());
            false
        }
    }

    /// Spawn this same executable with `--tcc-probe <id>` and parse its
    /// stdout handshake (`hope-agent-tcc-probe:granted=1|0`); anything else
    /// is unknown. Desktop-only: other runtimes may embed ha-core in
    /// binaries that do not implement the probe argument.
    fn fresh_process_probe(id: &str) -> Option<bool> {
        if !crate::app_init::is_desktop() {
            return None;
        }
        let exe = std::env::current_exe().ok()?;
        let mut child = Command::new(exe)
            .args(["--tcc-probe", id])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let deadline = Instant::now() + PROBE_WAIT;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
        // The child has exited (single-line output well under the pipe
        // buffer), so this read cannot block.
        let mut output = String::new();
        use std::io::Read as _;
        child.stdout.take()?.read_to_string(&mut output).ok()?;
        parse_probe_output(&output)
    }

    /// Decode the child's handshake line. Anything without the exact token —
    /// empty output from a binary that predates the flag, GUI/other-mode
    /// chatter, a malformed value — is `None` (unknown), never a grant.
    fn parse_probe_output(output: &str) -> Option<bool> {
        let value = output.lines().find_map(|line| {
            line.trim()
                .strip_prefix(crate::permissions::TCC_PROBE_OUTPUT_PREFIX)
        })?;
        match value {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        }
    }

    /// Drop the sticky pending-restart memory. MUST be called after a TCC
    /// reset: the positive is kept for the process lifetime on the premise
    /// that a grant cannot regress before relaunch, and a reset breaks
    /// exactly that premise — without this the panel would keep reporting
    /// "granted, restart to apply" for a grant that was just wiped.
    fn forget_screen_probe_memory() {
        if let Ok(mut state) = SCREEN_PROBE.lock() {
            state.pending_restart_confirmed = false;
            state.last_negative_at = None;
        }
    }

    /// `tccutil` service name for the permissions whose TCC record the user
    /// may reset from the UI. Deliberately a closed whitelist compiled into
    /// the binary: the service string must never originate outside this
    /// module, or the reset action would become "wipe any TCC service".
    fn tcc_reset_service(id: &str) -> Option<&'static str> {
        match id {
            "accessibility" => Some("Accessibility"),
            "screen_recording" => Some("ScreenCapture"),
            "input_monitoring" => Some("ListenEvent"),
            _ => None,
        }
    }

    pub fn supports_reset(id: &str) -> bool {
        tcc_reset_service(id).is_some() && bundle_identifier().is_some()
    }

    /// This app's `CFBundleIdentifier`, or `None` when the process is not
    /// running from a bundle (bare `target/debug/hope-agent`). Read from the
    /// running bundle rather than hardcoded, and the `None` case is load
    /// bearing: an unbundled process has no stable TCC identity, so there is
    /// nothing meaningful to reset and `tccutil` would either fail or hit
    /// some other bundle.
    fn bundle_identifier() -> Option<String> {
        let cls = objc_class(c"NSBundle")?;
        objc2::rc::autoreleasepool(|pool| unsafe {
            let bundle: *mut AnyObject = msg_send![cls, mainBundle];
            if bundle.is_null() {
                return None;
            }
            let identifier: *mut NSString = msg_send![bundle, bundleIdentifier];
            if identifier.is_null() {
                return None;
            }
            Some((*identifier).to_str(pool).to_owned())
        })
    }

    /// Reset this app's TCC record for `id` via `tccutil`, so macOS asks
    /// again on the next request. There is no public API for this — `tccutil`
    /// is the only supported route — but it needs no elevation for these
    /// services. Spawned with separate args (never a shell string).
    pub fn reset_item(id: &str) -> Result<(), String> {
        let Some(service) = tcc_reset_service(id) else {
            return Err(format!("Permission '{}' does not support reset.", id));
        };
        let Some(bundle_id) = bundle_identifier() else {
            return Err(
                "Resetting system permissions requires the packaged app (a bare development \
                 binary has no stable TCC identity)."
                    .to_string(),
            );
        };

        let output = Command::new("tccutil")
            .args(["reset", service, &bundle_id])
            .output()
            .map_err(|e| format!("Failed to run tccutil: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = stderr.trim();
            crate::app_warn!(
                "permissions",
                "tcc_reset",
                "{} reset failed: status={:?} stderr={}",
                id,
                output.status.code(),
                detail
            );
            return Err(if detail.is_empty() {
                format!("tccutil exited with status {:?}.", output.status.code())
            } else {
                detail.to_string()
            });
        }

        if id == "screen_recording" {
            forget_screen_probe_memory();
        }
        crate::app_info!(
            "permissions",
            "tcc_reset",
            "{} ({}) reset for {}",
            id,
            service,
            bundle_id
        );
        Ok(())
    }

    /// Raw, spawn-free preflight backing the `--tcc-probe` process mode.
    /// MUST never route through the fresh-process probe above — a probe
    /// child answering via another probe would recurse forever.
    pub fn raw_probe(id: &str) -> Option<bool> {
        match id {
            "accessibility" => Some(unsafe { AXIsProcessTrusted() }),
            "screen_recording" => Some(unsafe { CGPreflightScreenCaptureAccess() }),
            "input_monitoring" => Some(unsafe { CGPreflightListenEventAccess() }),
            _ => None,
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

    fn request_notifications(def: PermissionDef) -> SystemPermissionStatus {
        let status = notification_status();
        if let Some(status) = requestable_status_or_open(def, status) {
            return status;
        }
        if !running_from_app_bundle() {
            open_settings_pane(def.settings_pane);
            return SystemPermissionStatus::ManualCheck;
        }

        let Some(cls) = objc_class(c"UNUserNotificationCenter") else {
            return SystemPermissionStatus::NotApplicable;
        };
        let center: Retained<AnyObject> = unsafe { msg_send![cls, currentNotificationCenter] };
        let (sender, receiver) = mpsc::channel();
        let block = RcBlock::new(move |granted: Bool, _error: *mut AnyObject| {
            let _ = sender.send(bool_status(granted.as_bool()));
        });

        // UNAuthorizationOptionBadge | Sound | Alert
        let options: usize = (1 << 0) | (1 << 1) | (1 << 2);
        unsafe {
            let _: () = msg_send![
                &*center,
                requestAuthorizationWithOptions: options,
                completionHandler: &*block
            ];
        }

        wait_for_prompt(def.id, receiver)
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
        use super::{parse_probe_output, path_is_in_app_bundle};
        use std::path::Path;

        #[test]
        fn bundle_identifier_is_absent_outside_an_app_bundle() {
            // Exercises the NSBundle FFI (must not panic) and pins the
            // load-bearing None: the test harness is an unbundled binary, so
            // reset must be unavailable rather than aimed at some other
            // bundle's TCC record.
            assert_eq!(super::bundle_identifier(), None);
            assert!(!super::supports_reset("screen_recording"));
            assert!(super::reset_item("screen_recording").is_err());
        }

        #[test]
        fn only_whitelisted_permissions_map_to_a_tcc_service() {
            assert_eq!(
                super::tcc_reset_service("accessibility"),
                Some("Accessibility")
            );
            assert_eq!(
                super::tcc_reset_service("screen_recording"),
                Some("ScreenCapture")
            );
            assert_eq!(
                super::tcc_reset_service("input_monitoring"),
                Some("ListenEvent")
            );
            // Everything else — including real catalog ids and anything a
            // caller might inject — has no service and cannot be reset.
            for id in ["full_disk_access", "camera", "All", "", "ScreenCapture"] {
                assert_eq!(
                    super::tcc_reset_service(id),
                    None,
                    "unexpected mapping for {id:?}"
                );
            }
        }

        #[test]
        fn parses_probe_handshake_values() {
            assert_eq!(
                parse_probe_output("hope-agent-tcc-probe:granted=1\n"),
                Some(true)
            );
            assert_eq!(
                parse_probe_output("hope-agent-tcc-probe:granted=0\n"),
                Some(false)
            );
            assert_eq!(
                parse_probe_output("hope-agent-tcc-probe:granted=unknown\n"),
                None
            );
        }

        #[test]
        fn output_without_the_token_is_never_granted() {
            // A binary predating `--tcc-probe` prints nothing on this path
            // (or unrelated chatter) and can still exit 0 — e.g. a
            // single-instance forward. That must decode as unknown, never as
            // a grant, or the panel would claim a permission the user never
            // gave.
            assert_eq!(parse_probe_output(""), None);
            assert_eq!(parse_probe_output("Unknown arguments: --tcc-probe"), None);
            assert_eq!(parse_probe_output("granted=1"), None);
        }

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

    pub fn raw_probe(_id: &str) -> Option<bool> {
        None
    }

    pub fn supports_reset(_id: &str) -> bool {
        false
    }

    pub fn reset_item(_id: &str) -> Result<(), String> {
        Err("Resetting system permissions is only supported on macOS.".to_string())
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

    pub fn raw_probe(_id: &str) -> Option<bool> {
        None
    }

    pub fn supports_reset(_id: &str) -> bool {
        false
    }

    pub fn reset_item(_id: &str) -> Result<(), String> {
        Err("Resetting system permissions is only supported on macOS.".to_string())
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

    pub fn raw_probe(_id: &str) -> Option<bool> {
        None
    }

    pub fn supports_reset(_id: &str) -> bool {
        false
    }

    pub fn reset_item(_id: &str) -> Result<(), String> {
        Err("Resetting system permissions is only supported on macOS.".to_string())
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

pub(crate) fn raw_probe(id: &str) -> Option<bool> {
    imp::raw_probe(id)
}

pub(crate) fn supports_reset(id: &str) -> bool {
    imp::supports_reset(id)
}

pub(crate) fn reset_item(id: &str) -> Result<(), String> {
    imp::reset_item(id)
}
