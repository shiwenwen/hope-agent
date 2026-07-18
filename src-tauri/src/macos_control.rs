//! Desktop macOS control bridge.
//!
//! Registers the authorized desktop process and exposes Accessibility
//! snapshots, scored element search, display/window JPEG frames, app
//! launch/focus, window operations, AX-first element actions, dialogs, and menu
//! inspection/clicks.

#[cfg(target_os = "macos")]
mod imp {
    use std::collections::{BTreeMap, BTreeSet};
    use std::ffi::{CStr, CString};
    use std::fs;
    use std::os::raw::{c_char, c_void};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::ptr;
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    use async_trait::async_trait;
    use base64::Engine;
    use ha_core::mac_control::{
        mac_control_act_preview, normalize_perform_ax_action, MacControlActOp,
        MacControlActRequest, MacControlActResult, MacControlAppNameMatch, MacControlAppSummary,
        MacControlAppsOp, MacControlAppsRequest, MacControlAppsResult, MacControlBounds,
        MacControlBridge, MacControlClipboardOp, MacControlClipboardRequest,
        MacControlClipboardResult, MacControlDialogFileResult, MacControlDialogOp,
        MacControlDialogRequest, MacControlDialogResult, MacControlDialogSummary,
        MacControlDisplaySummary, MacControlDockItem, MacControlDockOp, MacControlDockRequest,
        MacControlDockResult, MacControlDockSection, MacControlElementCandidate,
        MacControlElementSummary, MacControlElementsRequest, MacControlElementsResult,
        MacControlFramePayload, MacControlInstalledApp, MacControlMenuItemSummary,
        MacControlMenuOp, MacControlMenuPopoverCandidate, MacControlMenuRequest,
        MacControlMenuResult, MacControlMenuScope, MacControlMotionProfile,
        MacControlOcrRawTextBlock, MacControlOcrRecognitionLevel, MacControlOcrRequest,
        MacControlRunningApp, MacControlScreenshotSummary, MacControlScreenshotTarget,
        MacControlSnapshot, MacControlSnapshotRequest, MacControlSpaceDirection,
        MacControlSpaceSummary, MacControlSpacesDisplay, MacControlSpacesOp,
        MacControlSpacesRequest, MacControlSpacesResult, MacControlStringMatch,
        MacControlTargetQuery, MacControlTypingProfile, MacControlVerification,
        MacControlVerificationCheck, MacControlVerificationStatus, MacControlWindowSummary,
        MacControlWindowsOp, MacControlWindowsRequest, MacControlWindowsResult,
        MacControlWindowsScope,
    };
    use image::codecs::jpeg::JpegEncoder;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
    use objc2::{sel, AnyThread};
    use objc2_app_kit::{
        NSApplicationActivationOptions, NSApplicationActivationPolicy, NSPasteboard,
        NSPasteboardItem, NSPasteboardWriting, NSRunningApplication, NSWorkspace,
    };
    use objc2_foundation::{NSArray, NSBundle, NSDictionary, NSString, NSURL};
    use objc2_vision::{
        VNImageOption, VNImageRequestHandler, VNRecognizeTextRequest, VNRequest,
        VNRequestTextRecognitionLevel,
    };
    use quick_xml::events::{BytesStart, Event};
    use quick_xml::Reader;
    use xcap::{Monitor, Window};

    struct TauriMacControlBridge;

    #[async_trait]
    impl MacControlBridge for TauriMacControlBridge {
        async fn system_permissions(&self) -> ha_core::permissions::SystemPermissionsResponse {
            ha_core::permissions::check_system_permissions().await
        }

        async fn snapshot(
            &self,
            request: MacControlSnapshotRequest,
        ) -> Result<MacControlSnapshot, String> {
            tokio::task::spawn_blocking(move || capture_ax_snapshot(request))
                .await
                .map_err(|e| format!("macOS snapshot worker failed: {e}"))?
        }

        async fn elements(
            &self,
            request: MacControlElementsRequest,
        ) -> Result<MacControlElementsResult, String> {
            tokio::task::spawn_blocking(move || handle_elements(request))
                .await
                .map_err(|e| format!("macOS elements worker failed: {e}"))?
        }

        async fn capture_frame(
            &self,
            display_id: Option<u32>,
        ) -> Result<MacControlFramePayload, String> {
            tokio::task::spawn_blocking(move || capture_desktop_frame(display_id))
                .await
                .map_err(|e| format!("macOS frame worker failed: {e}"))?
        }

        async fn list_displays(&self) -> Result<Vec<MacControlDisplaySummary>, String> {
            tokio::task::spawn_blocking(display_summaries)
                .await
                .map_err(|e| format!("macOS displays worker failed: {e}"))?
        }

        async fn apps(
            &self,
            request: MacControlAppsRequest,
        ) -> Result<MacControlAppsResult, String> {
            tokio::task::spawn_blocking(move || handle_apps(request))
                .await
                .map_err(|e| format!("macOS apps worker failed: {e}"))?
        }

        async fn dock(
            &self,
            request: MacControlDockRequest,
        ) -> Result<MacControlDockResult, String> {
            tokio::task::spawn_blocking(move || handle_dock(request))
                .await
                .map_err(|e| format!("macOS Dock worker failed: {e}"))?
        }

        async fn spaces(
            &self,
            request: MacControlSpacesRequest,
        ) -> Result<MacControlSpacesResult, String> {
            handle_spaces_on_main_thread(request).await
        }

        async fn windows(
            &self,
            request: MacControlWindowsRequest,
        ) -> Result<MacControlWindowsResult, String> {
            tokio::task::spawn_blocking(move || handle_windows(request))
                .await
                .map_err(|e| format!("macOS windows worker failed: {e}"))?
        }

        async fn act(&self, request: MacControlActRequest) -> Result<MacControlActResult, String> {
            tokio::task::spawn_blocking(move || handle_act(request))
                .await
                .map_err(|e| format!("macOS act worker failed: {e}"))?
        }

        async fn menu(
            &self,
            request: MacControlMenuRequest,
        ) -> Result<MacControlMenuResult, String> {
            tokio::task::spawn_blocking(move || handle_menu(request))
                .await
                .map_err(|e| format!("macOS menu worker failed: {e}"))?
        }

        async fn clipboard(
            &self,
            request: MacControlClipboardRequest,
        ) -> Result<MacControlClipboardResult, String> {
            tokio::task::spawn_blocking(move || handle_clipboard(request))
                .await
                .map_err(|e| format!("macOS clipboard worker failed: {e}"))?
        }

        async fn dialog(
            &self,
            request: MacControlDialogRequest,
        ) -> Result<MacControlDialogResult, String> {
            tokio::task::spawn_blocking(move || handle_dialog(request))
                .await
                .map_err(|e| format!("macOS dialog worker failed: {e}"))?
        }

        async fn ocr(
            &self,
            request: MacControlOcrRequest,
        ) -> Result<Vec<MacControlOcrRawTextBlock>, String> {
            tokio::task::spawn_blocking(move || handle_ocr(request))
                .await
                .map_err(|e| format!("macOS OCR worker failed: {e}"))?
        }
    }

    pub fn register() {
        let bridge: Arc<dyn MacControlBridge> = Arc::new(TauriMacControlBridge);
        ha_core::mac_control::set_mac_control_bridge(bridge);
    }

    type AXError = i32;
    type CFIndex = isize;
    type Boolean = u8;
    type CFTypeID = usize;
    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFArrayRef = *const c_void;
    type CFDataRef = *const c_void;
    type AXUIElementRef = *const c_void;
    type AXValueRef = *const c_void;
    type CGEventRef = *const c_void;
    type CGEventSourceRef = *const c_void;

    const AX_ERROR_SUCCESS: AXError = 0;
    const K_CFSTRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const K_CF_PROPERTY_LIST_XML_FORMAT_V1_0: u32 = 100;
    const K_CFNUMBER_SINT64_TYPE: i32 = 4;
    const K_AXVALUE_CGPOINT_TYPE: i32 = 1;
    const K_AXVALUE_CGSIZE_TYPE: i32 = 2;
    const K_AXVALUE_CGRECT_TYPE: i32 = 3;
    const K_CGS_ALL_SPACES_MASK: usize = (1 << 0) | (1 << 1) | (1 << 2);
    const K_CG_HID_EVENT_TAP: u32 = 0;
    const K_CG_EVENT_LEFT_MOUSE_DOWN: u32 = 1;
    const K_CG_EVENT_LEFT_MOUSE_UP: u32 = 2;
    const K_CG_EVENT_RIGHT_MOUSE_DOWN: u32 = 3;
    const K_CG_EVENT_RIGHT_MOUSE_UP: u32 = 4;
    const K_CG_EVENT_MOUSE_MOVED: u32 = 5;
    const K_CG_EVENT_LEFT_MOUSE_DRAGGED: u32 = 6;
    const K_CG_MOUSE_BUTTON_LEFT: u32 = 0;
    const K_CG_MOUSE_BUTTON_RIGHT: u32 = 1;
    const K_CG_MOUSE_EVENT_CLICK_STATE: u32 = 1;
    const K_CG_SCROLL_EVENT_UNIT_LINE: u32 = 1;
    const K_CG_EVENT_FLAG_MASK_SHIFT: u64 = 0x0002_0000;
    const K_CG_EVENT_FLAG_MASK_CONTROL: u64 = 0x0004_0000;
    const K_CG_EVENT_FLAG_MASK_ALTERNATE: u64 = 0x0008_0000;
    const K_CG_EVENT_FLAG_MASK_COMMAND: u64 = 0x0010_0000;
    const K_CG_EVENT_SOURCE_STATE_HID_SYSTEM_STATE: i32 = 1;
    const K_VK_COMMAND: u16 = 55;
    const K_VK_SHIFT: u16 = 56;
    const K_VK_OPTION: u16 = 58;
    const K_VK_CONTROL: u16 = 59;
    const VERIFY_SETTLE_MS: u64 = 120;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    #[repr(C)]
    struct CFArrayCallBacks {
        version: CFIndex,
        retain: *const c_void,
        release: *const c_void,
        copy_description: *const c_void,
        equal: *const c_void,
    }

    #[derive(Clone, Copy)]
    enum MouseButton {
        Left,
        Right,
    }

    #[derive(Clone, Copy)]
    struct HotkeyModifier {
        key_code: u16,
        flag: u64,
    }

    #[derive(Clone, Copy)]
    struct MotionProfile {
        steps: usize,
        duration_ms: u64,
        kind: MacControlMotionProfile,
    }

    #[derive(Clone, Copy)]
    struct TypingMotionProfile {
        base_delay_ms: u64,
        human_jitter: bool,
    }

    const DEFAULT_MOTION_STEPS: usize = 12;
    const DEFAULT_MOTION_DURATION_MS: u64 = 180;
    const DEFAULT_DRAG_STEPS: usize = 5;
    const DEFAULT_DRAG_DURATION_MS: u64 = 100;
    const STEADY_TYPING_DELAY_MS: u64 = 25;
    const HUMAN_TYPING_DELAY_MS: u64 = 45;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXUIElementCreateSystemWide() -> AXUIElementRef;
        fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> AXError;
        fn AXUIElementCopyActionNames(element: AXUIElementRef, names: *mut CFArrayRef) -> AXError;
        fn AXUIElementGetPid(element: AXUIElementRef, pid: *mut i32) -> AXError;
        fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> AXError;
        fn AXUIElementSetAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: CFTypeRef,
        ) -> AXError;
        fn AXValueCreate(value_type: i32, value: *const c_void) -> AXValueRef;
        fn AXValueGetType(value: AXValueRef) -> i32;
        fn AXValueGetValue(value: AXValueRef, value_type: i32, value: *mut c_void) -> Boolean;
        fn CGEventCreateMouseEvent(
            source: CGEventSourceRef,
            mouse_type: u32,
            mouse_cursor_position: CGPoint,
            mouse_button: u32,
        ) -> CGEventRef;
        fn CGEventCreateKeyboardEvent(
            source: CGEventSourceRef,
            virtual_key: u16,
            key_down: bool,
        ) -> CGEventRef;
        fn CGEventCreate(source: CGEventSourceRef) -> CGEventRef;
        fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
        fn CGEventSourceCreate(state_id: i32) -> CGEventSourceRef;
        fn CGEventCreateScrollWheelEvent(
            source: CGEventSourceRef,
            units: u32,
            wheel_count: u32,
            wheel1: i32,
            ...
        ) -> CGEventRef;
        fn CGEventSetFlags(event: CGEventRef, flags: u64);
        fn CGEventSetIntegerValueField(event: CGEventRef, field: u32, value: i64);
        fn CGEventKeyboardSetUnicodeString(
            event: CGEventRef,
            string_length: CFIndex,
            unicode_string: *const u16,
        );
        fn CGEventPost(tap: u32, event: CGEventRef);
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFBooleanTrue: CFTypeRef;
        static kCFTypeArrayCallBacks: CFArrayCallBacks;
        fn CFRelease(cf: CFTypeRef);
        fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
        fn CFEqual(cf1: CFTypeRef, cf2: CFTypeRef) -> Boolean;
        fn CFGetTypeID(cf: CFTypeRef) -> CFTypeID;
        fn CFStringGetTypeID() -> CFTypeID;
        fn CFArrayGetTypeID() -> CFTypeID;
        fn CFBooleanGetTypeID() -> CFTypeID;
        fn CFNumberGetTypeID() -> CFTypeID;
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            c_str: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFStringGetLength(the_string: CFStringRef) -> CFIndex;
        fn CFStringGetMaximumSizeForEncoding(length: CFIndex, encoding: u32) -> CFIndex;
        fn CFStringGetCString(
            the_string: CFStringRef,
            buffer: *mut c_char,
            buffer_size: CFIndex,
            encoding: u32,
        ) -> Boolean;
        fn CFArrayGetCount(the_array: CFArrayRef) -> CFIndex;
        fn CFArrayGetValueAtIndex(the_array: CFArrayRef, idx: CFIndex) -> *const c_void;
        fn CFArrayCreate(
            allocator: *const c_void,
            values: *const CFTypeRef,
            num_values: CFIndex,
            callbacks: *const CFArrayCallBacks,
        ) -> CFArrayRef;
        fn CFNumberCreate(
            allocator: *const c_void,
            the_type: i32,
            value_ptr: *const c_void,
        ) -> CFTypeRef;
        fn CFNumberGetValue(number: CFTypeRef, the_type: i32, value_ptr: *mut c_void) -> Boolean;
        fn CFBooleanGetValue(boolean: CFTypeRef) -> Boolean;
        fn CFPropertyListCreateData(
            allocator: *const c_void,
            property_list: CFTypeRef,
            format: u32,
            options: u64,
            error: *mut CFTypeRef,
        ) -> CFDataRef;
        fn CFDataGetBytePtr(the_data: CFDataRef) -> *const u8;
        fn CFDataGetLength(the_data: CFDataRef) -> CFIndex;
        fn CFErrorCopyDescription(error: CFTypeRef) -> CFStringRef;
    }

    struct CfOwned(CFTypeRef);

    impl CfOwned {
        fn new(ptr: CFTypeRef) -> Option<Self> {
            if ptr.is_null() {
                None
            } else {
                Some(Self(ptr))
            }
        }

        fn as_ptr(&self) -> CFTypeRef {
            self.0
        }
    }

    impl Drop for CfOwned {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0) };
        }
    }

    struct CaptureState {
        max_elements: usize,
        max_depth: usize,
        next_element_id: usize,
        elements: Vec<MacControlElementSummary>,
        truncated: bool,
    }

    struct FocusedOverlayRoot {
        element: CfOwned,
        window_id: Option<String>,
    }

    struct WebAreaCandidate {
        element: CfOwned,
        area: f64,
        focused: bool,
        has_text_input: bool,
    }

    struct CapturedDesktopFrame {
        jpeg: Vec<u8>,
        width_px: u32,
        height_px: u32,
        target: MacControlScreenshotTarget,
        display_id: Option<u32>,
        window_id: Option<String>,
        window_title: Option<String>,
        bounds_points: Option<MacControlBounds>,
        scale: Option<f64>,
    }

    #[derive(Clone)]
    struct MenuPopoverOcrBlock {
        text: String,
        screen_bounds: MacControlBounds,
    }

    fn capture_ax_snapshot(
        request: MacControlSnapshotRequest,
    ) -> Result<MacControlSnapshot, String> {
        let request = request.clamped();
        let system = unsafe { AXUIElementCreateSystemWide() };
        let system = CfOwned::new(system as CFTypeRef)
            .ok_or_else(|| "Unable to create the system Accessibility element.".to_string())?;
        let app = copy_attribute(system.as_ptr() as AXUIElementRef, "AXFocusedApplication")
            .ok_or_else(|| {
                "Unable to read the focused macOS application via Accessibility.".to_string()
            })?;
        let app_ref = app.as_ptr() as AXUIElementRef;

        let mut snapshot = MacControlSnapshot::new_empty();
        snapshot.frontmost_app = Some(app_summary(app_ref));
        match display_summaries() {
            Ok(displays) => snapshot.displays = displays,
            Err(error) => snapshot.warnings.push(error),
        }

        populate_ax_snapshot_tree(app_ref, &request, &mut snapshot);
        maybe_focus_web_area_and_repopulate(app_ref, &request, &mut snapshot);
        if snapshot.truncated {
            snapshot.warnings.push(
                "AX snapshot was truncated; increase maxElements/maxDepth for more context."
                    .to_string(),
            );
        }
        if request.include_screenshot {
            match capture_desktop_frame_with_id(&snapshot, &request) {
                Ok((frame, screenshot)) => {
                    snapshot.screenshot = Some(screenshot);
                    ha_core::mac_control::emit_frame(&frame);
                }
                Err(error) => snapshot.warnings.push(format!(
                    "Screenshot capture failed; returning AX-only snapshot: {error}"
                )),
            }
        }
        Ok(snapshot)
    }

    fn populate_ax_snapshot_tree(
        app_ref: AXUIElementRef,
        request: &MacControlSnapshotRequest,
        snapshot: &mut MacControlSnapshot,
    ) {
        snapshot.windows.clear();
        snapshot.elements.clear();
        snapshot.truncated = false;

        let mut state = CaptureState {
            max_elements: request.max_elements,
            max_depth: request.max_depth,
            next_element_id: 1,
            elements: Vec::new(),
            truncated: false,
        };

        traverse_focused_overlay_roots(app_ref, &mut state);

        if let Some(windows) = copy_attribute(app_ref, "AXWindows") {
            for (idx, window_ref) in cf_array_values(windows.as_ptr()).into_iter().enumerate() {
                if state.truncated {
                    break;
                }
                let window = window_ref as AXUIElementRef;
                let window_id = format!("win_{}", idx + 1);
                snapshot.windows.push(window_summary(window, &window_id));
                traverse_element(window, 0, Some(&window_id), &mut state);
                if state.truncated {
                    break;
                }
            }
        }

        if snapshot.windows.is_empty() {
            traverse_element(app_ref, 0, None, &mut state);
        }

        snapshot.elements = state.elements;
        snapshot.truncated = state.truncated;
    }

    fn maybe_focus_web_area_and_repopulate(
        app_ref: AXUIElementRef,
        request: &MacControlSnapshotRequest,
        snapshot: &mut MacControlSnapshot,
    ) {
        if !snapshot.elements.iter().any(element_role_is_web_area) {
            return;
        }
        if snapshot.elements.iter().any(focused_text_input_element) {
            return;
        }
        let Some(candidate) = dominant_web_area(app_ref, request.max_depth) else {
            return;
        };
        if candidate.focused || candidate.has_text_input {
            return;
        }
        match set_ax_bool(
            candidate.element.as_ptr() as AXUIElementRef,
            "AXFocused",
            true,
        ) {
            Ok(()) => {
                thread::sleep(Duration::from_millis(80));
                populate_ax_snapshot_tree(app_ref, request, snapshot);
                snapshot.warnings.push(
                    "Focused dominant AXWebArea and re-traversed Accessibility tree because web content did not expose text inputs."
                        .to_string(),
                );
            }
            Err(error) => snapshot.warnings.push(format!(
                "AXWebArea focus fallback was skipped because focusing failed: {error}"
            )),
        }
    }

    fn dominant_web_area(app_ref: AXUIElementRef, max_depth: usize) -> Option<WebAreaCandidate> {
        let mut best = None;
        let search_depth = max_depth.max(8);
        if let Some(windows) = copy_attribute(app_ref, "AXWindows") {
            for window_ref in cf_array_values(windows.as_ptr()) {
                collect_dominant_web_area(window_ref as AXUIElementRef, 0, search_depth, &mut best);
            }
        }
        if best.is_none() {
            collect_dominant_web_area(app_ref, 0, search_depth, &mut best);
        }
        best
    }

    fn collect_dominant_web_area(
        element: AXUIElementRef,
        depth: usize,
        max_depth: usize,
        best: &mut Option<WebAreaCandidate>,
    ) {
        let summary = element_summary(element, None, 0);
        if element_role_is_web_area(&summary) {
            let area = summary
                .bounds_points
                .map(|bounds| bounds.width.max(0.0) * bounds.height.max(0.0))
                .unwrap_or(0.0);
            let score = area + if summary.focused { 1_000_000.0 } else { 0.0 };
            let best_score = best
                .as_ref()
                .map(|candidate| candidate.area + if candidate.focused { 1_000_000.0 } else { 0.0 })
                .unwrap_or(-1.0);
            if score > best_score {
                if let Some(retained) = CfOwned::new(unsafe { CFRetain(element as CFTypeRef) }) {
                    *best = Some(WebAreaCandidate {
                        element: retained,
                        area,
                        focused: summary.focused,
                        has_text_input: web_area_has_text_input(
                            element,
                            0,
                            max_depth.saturating_sub(depth),
                        ),
                    });
                }
            }
        }
        if depth >= max_depth {
            return;
        }
        let Some(children) = copy_attribute(element, "AXChildren")
            .or_else(|| copy_attribute(element, "AXVisibleChildren"))
        else {
            return;
        };
        for child_ref in cf_array_values(children.as_ptr()) {
            collect_dominant_web_area(child_ref as AXUIElementRef, depth + 1, max_depth, best);
        }
    }

    fn web_area_has_text_input(element: AXUIElementRef, depth: usize, max_depth: usize) -> bool {
        if depth > 0 && is_text_input_element(&element_summary(element, None, 0)) {
            return true;
        }
        if depth >= max_depth {
            return false;
        }
        let Some(children) = copy_attribute(element, "AXChildren")
            .or_else(|| copy_attribute(element, "AXVisibleChildren"))
        else {
            return false;
        };
        cf_array_values(children.as_ptr())
            .into_iter()
            .any(|child_ref| {
                web_area_has_text_input(child_ref as AXUIElementRef, depth + 1, max_depth)
            })
    }

    fn handle_apps(request: MacControlAppsRequest) -> Result<MacControlAppsResult, String> {
        let request = request.clamped();
        let workspace = NSWorkspace::sharedWorkspace();
        let initial_frontmost = workspace
            .frontmostApplication()
            .as_deref()
            .map(running_app_summary);
        let running = workspace.runningApplications().to_vec();
        let mut all_apps = running
            .iter()
            .map(|app| running_app_summary(app))
            .collect::<Vec<_>>();
        if let Some(frontmost) = initial_frontmost {
            merge_running_app_summary(&mut all_apps, frontmost);
        }
        if let Some(bundle_id) = request
            .bundle_id
            .as_deref()
            .filter(|bundle_id| !bundle_id.is_empty())
        {
            for app in running_apps_with_bundle_id(bundle_id) {
                merge_running_app_summary(&mut all_apps, running_app_summary(&app));
            }
        }

        if all_apps.len() <= 1
            || (matches!(
                request.op,
                MacControlAppsOp::Activate | MacControlAppsOp::Quit
            ) && !all_apps
                .iter()
                .any(|app| app_matches_request(app, &request)))
        {
            for app in fallback_running_app_summaries() {
                merge_running_app_summary(&mut all_apps, app);
            }
        }

        let mut apps = all_apps
            .iter()
            .filter(|app| app_matches_request(app, &request))
            .take(request.limit)
            .cloned()
            .collect::<Vec<_>>();
        let installed_apps = if matches!(
            request.op,
            MacControlAppsOp::Installed | MacControlAppsOp::Search
        ) {
            installed_apps_for_request(&request, &running)
        } else {
            Vec::new()
        };

        let mut launched = None;
        let mut quit = None;
        let mut execution = None;
        let activated = match request.op {
            MacControlAppsOp::Activate => {
                let app = find_running_app_for_request(&request, &running, &all_apps).ok_or_else(
                    || "No running macOS app matched the activate request.".to_string(),
                )?;
                activate_running_app(&app)?;
                let summary = running_app_summary(&app);
                if apps.iter().all(|item| item.pid != summary.pid) {
                    apps.insert(0, summary.clone());
                }
                Some(summary)
            }
            MacControlAppsOp::Launch => {
                let app = launch_app(&request)?;
                let summary = running_app_summary(&app);
                if apps.iter().all(|item| item.pid != summary.pid) {
                    apps.insert(0, summary.clone());
                }
                launched = Some(summary.clone());
                activate_running_app(&app)?;
                Some(summary)
            }
            MacControlAppsOp::Quit => {
                let app = find_running_app_for_request(&request, &running, &all_apps)
                    .ok_or_else(|| "No running macOS app matched the quit request.".to_string())?;
                let summary = running_app_summary(&app);
                if summary.pid as u32 == std::process::id() {
                    return Err("apps.quit cannot quit Hope Agent through mac_control.".to_string());
                }
                let method = quit_running_app(&app, &summary)?;
                if apps.iter().all(|item| item.pid != summary.pid) {
                    apps.insert(0, summary.clone());
                }
                quit = Some(summary);
                execution = Some(method);
                None
            }
            MacControlAppsOp::List
            | MacControlAppsOp::Frontmost
            | MacControlAppsOp::Installed
            | MacControlAppsOp::Search => None,
        };

        let frontmost = workspace
            .frontmostApplication()
            .as_deref()
            .map(running_app_summary);

        Ok(MacControlAppsResult {
            op: request.op,
            frontmost,
            apps,
            installed_apps,
            activated,
            launched,
            quit,
            execution,
        })
    }

    fn handle_dock(request: MacControlDockRequest) -> Result<MacControlDockResult, String> {
        let request = request.clamped();
        let mut dock = read_dock_state()?;
        let mut launched = None;
        let mut menu_items = Vec::new();
        let mut selected_menu_item = None;
        let mut execution = None;
        let mut warnings = Vec::new();

        match request.op {
            MacControlDockOp::List => {}
            MacControlDockOp::Launch => {
                let item = resolve_dock_item(&dock.items, &request)?.clone();
                launch_dock_item(&item)?;
                execution = Some("NSWorkspace.openURL".to_string());
                launched = Some(item);
                dock = read_dock_state().unwrap_or_else(|error| {
                    warnings.push(format!("Unable to refresh Dock after launch: {error}"));
                    dock
                });
            }
            MacControlDockOp::Menu => {
                let item = resolve_dock_item(&dock.items, &request)?.clone();
                let element = resolve_dock_ax_item(&item, &request)?;
                let (opened_items, method) =
                    open_dock_context_menu(element.as_ptr() as AXUIElementRef)?;
                execution = Some(method);
                menu_items = opened_items;
            }
            MacControlDockOp::SelectMenu => {
                let item = resolve_dock_item(&dock.items, &request)?.clone();
                let element = resolve_dock_ax_item(&item, &request)?;
                let (opened_items, method) =
                    open_dock_context_menu(element.as_ptr() as AXUIElementRef)?;
                let selected = select_dock_context_menu_item(&request)?;
                execution = Some(format!("{method}+AXPress"));
                menu_items = opened_items;
                selected_menu_item = Some(selected);
                dock = read_dock_state().unwrap_or_else(|error| {
                    warnings.push(format!(
                        "Unable to refresh Dock after context menu selection: {error}"
                    ));
                    dock
                });
            }
            MacControlDockOp::Hide => {
                set_dock_autohide(true)?;
                execution = Some("defaults.write+killall.Dock".to_string());
                dock.autohide = Some(true);
            }
            MacControlDockOp::Show => {
                set_dock_autohide(false)?;
                execution = Some("defaults.write+killall.Dock".to_string());
                dock.autohide = Some(false);
            }
        }

        let items = dock
            .items
            .into_iter()
            .filter(|item| dock_item_matches_request(item, &request))
            .take(request.limit)
            .collect();

        Ok(MacControlDockResult {
            op: request.op,
            autohide: dock.autohide,
            orientation: dock.orientation,
            items,
            launched,
            menu_items,
            selected_menu_item,
            execution,
            warnings,
        })
    }

    async fn handle_spaces_on_main_thread(
        request: MacControlSpacesRequest,
    ) -> Result<MacControlSpacesResult, String> {
        let app = crate::globals::get_app_handle()
            .ok_or_else(|| "Tauri AppHandle is unavailable for macOS Spaces control.".to_string())?
            .clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.run_on_main_thread(move || {
            let _ = tx.send(handle_spaces(request));
        })
        .map_err(|error| {
            format!("Failed to dispatch macOS Spaces control to main thread: {error}")
        })?;
        rx.await
            .map_err(|_| "macOS Spaces main-thread worker was canceled.".to_string())?
    }

    fn handle_spaces(request: MacControlSpacesRequest) -> Result<MacControlSpacesResult, String> {
        let request = request.clamped();
        let mut spaces = read_spaces_state()?;
        let mut switched = None;
        let mut moved_window = None;
        let mut execution = None;
        let mut warnings = std::mem::take(&mut spaces.warnings);

        match request.op {
            MacControlSpacesOp::List => {}
            MacControlSpacesOp::Switch => {
                if request.direction.is_some() {
                    let before = current_space_from_displays(&spaces.displays);
                    let (keys, label) = spaces_switch_hotkey(&request, &spaces.displays)?;
                    let label = label.replacen("CGEventHotkey", "MissionControlHotkey", 1);
                    execution = Some(post_mission_control_hotkey(&keys, &label, &mut warnings)?);
                    thread::sleep(Duration::from_millis(650));
                    refresh_spaces_after_action(&mut spaces, &mut switched, &mut warnings);
                    if same_space_summary(before.as_ref(), switched.as_ref()) {
                        warnings.push(
                            "Mission Control hotkey did not change the current Space; the visible desktop may already be at that edge, or the macOS shortcut is unavailable to Hope Agent."
                                .to_string(),
                        );
                    }
                } else {
                    let target = resolve_spaces_switch_target(&request, &spaces.displays)?;
                    let mut mission_control_hotkey = None;
                    if let Some((keys, label)) =
                        preferred_spaces_switch_hotkey(&request, &spaces.displays, &target.space)
                    {
                        if keys.is_empty() {
                            execution = Some(label);
                        } else {
                            execution =
                                Some(post_mission_control_hotkey(&keys, &label, &mut warnings)?);
                            mission_control_hotkey = Some(keys);
                        }
                    } else if let Some(space_id) = target.space.id {
                        match switch_visible_space_with_cgs(space_id) {
                            Ok(()) => {
                                execution = Some(format!(
                                    "CGSManagedDisplaySetCurrentSpace kCGSPackagesMainDisplayIdentifier space={space_id}"
                                ));
                            }
                            Err(error) => {
                                warnings.push(format!(
                                    "CGS Spaces switch failed; falling back to Mission Control hotkey: {error}"
                                ));
                                let (keys, label) =
                                    spaces_switch_hotkey(&request, &spaces.displays)?;
                                let label = format!(
                                    "{} fallback",
                                    label.replacen("CGEventHotkey", "MissionControlHotkey", 1)
                                );
                                execution = Some(post_mission_control_hotkey(
                                    &keys,
                                    &label,
                                    &mut warnings,
                                )?);
                                mission_control_hotkey = Some(keys);
                            }
                        }
                    } else {
                        let (keys, label) = spaces_switch_hotkey(&request, &spaces.displays)?;
                        let label = format!(
                            "{} fallback",
                            label.replacen("CGEventHotkey", "MissionControlHotkey", 1)
                        );
                        execution =
                            Some(post_mission_control_hotkey(&keys, &label, &mut warnings)?);
                        mission_control_hotkey = Some(keys);
                        warnings.push(
                            "Target Space has no ManagedSpaceID; fell back to Mission Control hotkey."
                                .to_string(),
                        );
                    }
                    thread::sleep(Duration::from_millis(650));
                    let mut matched = refresh_spaces_after_switch(
                        &mut spaces,
                        &mut switched,
                        &mut warnings,
                        &target.space,
                    );
                    if !matched && mission_control_hotkey.is_some() {
                        if let Some(space_id) = target.space.id {
                            warnings.push(
                                "Mission Control hotkey was sent, but the current Space could not be verified as the requested target; trying CGS fallback."
                                    .to_string(),
                            );
                            match switch_visible_space_with_cgs(space_id) {
                                Ok(()) => {
                                    execution = Some(format!(
                                        "{} -> CGSManagedDisplaySetCurrentSpace fallback space={space_id}",
                                        execution.as_deref().unwrap_or("MissionControlHotkey")
                                    ));
                                    thread::sleep(Duration::from_millis(650));
                                    matched = refresh_spaces_after_switch(
                                        &mut spaces,
                                        &mut switched,
                                        &mut warnings,
                                        &target.space,
                                    );
                                }
                                Err(error) => warnings.push(format!(
                                    "CGS fallback after Mission Control hotkey failed: {error}"
                                )),
                            }
                        } else {
                            warnings.push(
                                "Mission Control hotkey was sent, but the current Space could not be verified and the target has no ManagedSpaceID for CGS fallback."
                                    .to_string(),
                            );
                        }
                    }
                    if !matched && mission_control_hotkey.is_some() {
                        warnings.push(
                            "Mission Control hotkey/CGS fallback did not verify the requested Space."
                                .to_string(),
                        );
                    }
                    if !space_matches_summary(switched.as_ref(), &target.space) {
                        warnings.push(format!(
                            "Requested Space index={} id={:?}, but current Space after switch is index={} id={:?}.",
                            target.space.index,
                            target.space.id,
                            switched.as_ref().map(|space| space.index).unwrap_or_default(),
                            switched.as_ref().and_then(|space| space.id)
                        ));
                    } else if !matched {
                        warnings.push(
                            "Spaces state matched the requested target only after fallback refresh."
                                .to_string(),
                        );
                    }
                }
            }
            MacControlSpacesOp::MoveWindow => {
                let target = resolve_spaces_switch_target(&request, &spaces.displays)?;
                let target_space_id = target.space.id.ok_or_else(|| {
                    format!(
                        "Target Space index={} has no ManagedSpaceID; spaces.move_window requires live CGS Spaces state.",
                        target.space.index
                    )
                })?;
                let window_request = MacControlWindowsRequest {
                    op: MacControlWindowsOp::List,
                    window_scope: MacControlWindowsScope::All,
                    target: request.target.clone(),
                    window_id: request.window_id.clone(),
                    max_elements: request.max_elements,
                    max_depth: request.max_depth,
                    ..Default::default()
                };
                let (window, summary) = resolve_window(&window_request)?;
                let cg_window_id = ax_window_number(window.as_ptr() as AXUIElementRef)
                    .map_or_else(
                        || {
                            let cg_window = find_xcap_window_for_summary(&summary)?;
                            cg_window.id().map_err(|error| {
                                format!("Unable to read CGWindowID for '{}': {error}", summary.id)
                            })
                        },
                        Ok,
                    )?;
                let previous_spaces = move_window_to_space_with_cgs(cg_window_id, target_space_id)?;
                thread::sleep(Duration::from_millis(150));
                match cgs_spaces_for_window(cg_window_id) {
                    Ok(space_ids) if space_ids.contains(&target_space_id) => {}
                    Ok(space_ids) => warnings.push(format!(
                        "CGS moved window {}, but post-move verification reported Spaces {:?} instead of target Space {}.",
                        cg_window_id, space_ids, target_space_id
                    )),
                    Err(error) => warnings.push(format!(
                        "CGS moved window {}, but post-move verification failed: {error}",
                        cg_window_id
                    )),
                }
                execution = Some(format!(
                    "CGSRemoveWindowsFromSpaces+CGSAddWindowsToSpaces window={} from={:?} to={}",
                    cg_window_id, previous_spaces, target_space_id
                ));
                moved_window = Some(summary);
                refresh_spaces_after_action(&mut spaces, &mut switched, &mut warnings);
            }
        }

        Ok(MacControlSpacesResult {
            op: request.op,
            displays: spaces.displays,
            switched,
            moved_window,
            execution,
            warnings,
        })
    }

    #[derive(Debug, Clone)]
    struct DockState {
        autohide: Option<bool>,
        orientation: Option<String>,
        items: Vec<MacControlDockItem>,
    }

    #[derive(Debug, Clone)]
    struct SpacesState {
        displays: Vec<MacControlSpacesDisplay>,
        warnings: Vec<String>,
    }

    struct SpacesSwitchTarget {
        space: MacControlSpaceSummary,
    }

    #[derive(Debug, Clone)]
    enum PlistValue {
        Dict(BTreeMap<String, PlistValue>),
        Array(Vec<PlistValue>),
        String(String),
        Integer(i64),
        Real,
        Bool(bool),
        Data,
    }

    fn read_dock_state() -> Result<DockState, String> {
        let root = read_defaults_domain("com.apple.dock")?;
        let autohide = plist_get(&root, "autohide").and_then(plist_bool);
        let orientation = plist_get(&root, "orientation")
            .and_then(plist_string)
            .map(ToString::to_string);
        let running = NSWorkspace::sharedWorkspace()
            .runningApplications()
            .to_vec();
        let mut items = Vec::new();
        append_dock_items(
            &mut items,
            plist_get(&root, "persistent-apps"),
            MacControlDockSection::PersistentApps,
            &running,
        );
        append_dock_items(
            &mut items,
            plist_get(&root, "persistent-others"),
            MacControlDockSection::PersistentOthers,
            &running,
        );
        Ok(DockState {
            autohide,
            orientation,
            items,
        })
    }

    fn append_dock_items(
        out: &mut Vec<MacControlDockItem>,
        value: Option<&PlistValue>,
        section: MacControlDockSection,
        running: &[Retained<NSRunningApplication>],
    ) {
        let Some(items) = value.and_then(plist_array) else {
            return;
        };
        for item in items {
            let Some(dict) = plist_dict(item) else {
                continue;
            };
            let tile_data = dict.get("tile-data").and_then(plist_dict);
            let guid = dict.get("GUID").and_then(plist_integer);
            let tile_type = dict
                .get("tile-type")
                .and_then(plist_string)
                .map(ToString::to_string);
            let label = tile_data
                .and_then(|tile| tile.get("file-label"))
                .and_then(plist_string)
                .map(ToString::to_string);
            let bundle_id = tile_data
                .and_then(|tile| tile.get("bundle-identifier"))
                .and_then(plist_string)
                .map(ToString::to_string);
            let path = tile_data
                .and_then(|tile| tile.get("file-data"))
                .and_then(plist_dict)
                .and_then(|file_data| file_data.get("_CFURLString"))
                .and_then(plist_string)
                .and_then(dock_url_to_path);
            let running_app =
                running_app_for_installed(bundle_id.as_deref(), path.as_deref(), running);
            let running_summary = running_app.as_deref().map(running_app_summary);
            let index = out.len() + 1;
            out.push(MacControlDockItem {
                id: guid
                    .map(|guid| format!("dock_{guid}"))
                    .unwrap_or_else(|| format!("dock_{index}")),
                index,
                section,
                tile_type,
                label: running_summary
                    .as_ref()
                    .and_then(|app| app.name.clone())
                    .or(label),
                bundle_id: bundle_id.or_else(|| {
                    running_summary
                        .as_ref()
                        .and_then(|app| app.bundle_id.clone())
                }),
                path,
                running: running_summary.is_some(),
                pid: running_summary.as_ref().map(|app| app.pid),
                active: running_summary.as_ref().is_some_and(|app| app.active),
                hidden: running_summary.as_ref().is_some_and(|app| app.hidden),
            });
        }
    }

    fn resolve_dock_item<'a>(
        items: &'a [MacControlDockItem],
        request: &MacControlDockRequest,
    ) -> Result<&'a MacControlDockItem, String> {
        let matches = items
            .iter()
            .filter(|item| dock_item_matches_request(item, request))
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => Err("No Dock item matched the request.".to_string()),
            [item] => Ok(item),
            _ => Err(format!(
                "Dock request matched {} items; retry with dockItemId or bundleId.",
                matches.len()
            )),
        }
    }

    fn dock_item_matches_request(
        item: &MacControlDockItem,
        request: &MacControlDockRequest,
    ) -> bool {
        if request
            .dock_item_id
            .as_deref()
            .is_some_and(|id| item.id != id)
        {
            return false;
        }
        if !contains_ci(item.bundle_id.as_deref(), request.bundle_id.as_deref()) {
            return false;
        }
        if request
            .item_path
            .as_deref()
            .is_some_and(|path| item.path.as_deref() != Some(path))
        {
            return false;
        }
        if let Some(app_name) = request.app_name.as_deref() {
            let path_name = item
                .path
                .as_deref()
                .and_then(|path| app_bundle_name(Path::new(path)));
            return app_name_matches_values(
                request.app_name_match,
                app_name,
                [
                    item.label.as_deref(),
                    item.bundle_id
                        .as_deref()
                        .and_then(|bundle_id| bundle_id.rsplit('.').next()),
                    path_name.as_deref(),
                    item.bundle_id.as_deref(),
                ],
            );
        }
        true
    }

    fn resolve_dock_ax_item(
        item: &MacControlDockItem,
        request: &MacControlDockRequest,
    ) -> Result<CfOwned, String> {
        let mut elements = dock_ax_items()?;
        let mut best: Option<(i64, usize)> = None;
        for (idx, element) in elements.iter().enumerate() {
            let score = dock_ax_item_score(element.as_ptr() as AXUIElementRef, item, request);
            if score <= 0 {
                continue;
            }
            if best
                .as_ref()
                .is_none_or(|(best_score, _)| score > *best_score)
            {
                best = Some((score, idx));
            }
        }
        if let Some((_, idx)) = best {
            return Ok(elements.remove(idx));
        }
        let fallback_index = item.index.saturating_sub(1);
        if fallback_index < elements.len() {
            return Ok(elements.remove(fallback_index));
        }
        Err(format!(
            "Unable to match Dock item '{}'{} to the live Dock AX list.",
            item.id,
            item.label
                .as_deref()
                .map(|label| format!(" ({label})"))
                .unwrap_or_default()
        ))
    }

    fn dock_ax_items() -> Result<Vec<CfOwned>, String> {
        let app = running_apps_with_bundle_id("com.apple.dock")
            .into_iter()
            .next()
            .ok_or_else(|| "Dock application is not running.".to_string())?;
        let app_element = app_element_for_pid(app.processIdentifier())
            .ok_or_else(|| "Unable to create Accessibility element for Dock.".to_string())?;
        let dock_list = direct_menu_children(app_element.as_ptr() as AXUIElementRef)
            .into_iter()
            .find(|child| {
                attribute_string(child.as_ptr() as AXUIElementRef, "AXRole").as_deref()
                    == Some("AXList")
            })
            .ok_or_else(|| "Dock AXList was not found.".to_string())?;
        Ok(direct_menu_children(dock_list.as_ptr() as AXUIElementRef))
    }

    fn dock_ax_item_score(
        element: AXUIElementRef,
        item: &MacControlDockItem,
        request: &MacControlDockRequest,
    ) -> i64 {
        let values = menu_item_match_strings(element);
        let mut score = 0_i64;
        for expected in [
            item.label.as_deref(),
            request.app_name.as_deref(),
            item.path
                .as_deref()
                .and_then(|path| app_bundle_name(Path::new(path)))
                .as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if values
                .iter()
                .any(|value| value.eq_ignore_ascii_case(expected))
            {
                score = score.max(1_000);
            } else if values
                .iter()
                .any(|value| contains_ci(Some(value), Some(expected)))
            {
                score = score.max(500);
            }
        }
        score
    }

    fn open_dock_context_menu(
        element: AXUIElementRef,
    ) -> Result<(Vec<MacControlMenuItemSummary>, String), String> {
        let method = if action_names(element)
            .iter()
            .any(|action| action == "AXShowMenu")
            && perform_ax_action(element, "AXShowMenu").is_ok()
        {
            "AXShowMenu".to_string()
        } else {
            let bounds = element_bounds(element)
                .ok_or_else(|| "Dock item has no bounds for right-click fallback.".to_string())?;
            post_mouse_click(center_point(bounds, "Dock item")?, MouseButton::Right)?;
            "CGEventRightClick".to_string()
        };
        thread::sleep(Duration::from_millis(300));
        let menu = dock_context_menu(element)?;
        let mut items = menu_children(menu.as_ptr() as AXUIElementRef, 2);
        assign_menu_item_metadata(&mut items, "dock_menu");
        Ok((items, method))
    }

    fn select_dock_context_menu_item(
        request: &MacControlDockRequest,
    ) -> Result<MacControlMenuItemSummary, String> {
        let menu = dock_context_menu(ptr::null())?;
        let item_elements = direct_menu_children(menu.as_ptr() as AXUIElementRef);
        let (idx, item) = if let Some(menu_item) = request.menu_item.as_deref() {
            let (idx, item) = item_elements
                .iter()
                .enumerate()
                .find(|(_, item)| {
                    menu_item_matches_exact(item.as_ptr() as AXUIElementRef, menu_item)
                })
                .or_else(|| {
                    item_elements.iter().enumerate().find(|(_, item)| {
                        menu_item_matches_contains(item.as_ptr() as AXUIElementRef, menu_item)
                    })
                })
                .ok_or_else(|| format!("Dock context menu item '{menu_item}' was not found."))?;
            (idx, item.as_ptr() as AXUIElementRef)
        } else if let Some(index) = request.menu_index {
            let item = item_elements.get(index).ok_or_else(|| {
                format!(
                    "Dock context menu index {index} was not found; valid range is 0..{}.",
                    item_elements.len().saturating_sub(1)
                )
            })?;
            (index, item.as_ptr() as AXUIElementRef)
        } else {
            return Err("dock.select_menu requires menuItem or menuIndex.".to_string());
        };
        let mut summary = menu_item_summary(item, 2);
        summary.index = Some(idx);
        summary.id = Some(format!("dock_menu_{}", idx + 1));
        perform_menu_click_action(item)?;
        Ok(summary)
    }

    fn dock_context_menu(preferred_parent: AXUIElementRef) -> Result<CfOwned, String> {
        if !preferred_parent.is_null() {
            if let Some(menu) = direct_menu_children(preferred_parent)
                .into_iter()
                .find(|child| {
                    attribute_string(child.as_ptr() as AXUIElementRef, "AXRole").as_deref()
                        == Some("AXMenu")
                })
            {
                return Ok(menu);
            }
        }
        let system = unsafe { AXUIElementCreateSystemWide() };
        let system = CfOwned::new(system as CFTypeRef)
            .ok_or_else(|| "Unable to create system Accessibility element.".to_string())?;
        direct_menu_children(system.as_ptr() as AXUIElementRef)
            .into_iter()
            .find(|child| {
                attribute_string(child.as_ptr() as AXUIElementRef, "AXRole").as_deref()
                    == Some("AXMenu")
            })
            .ok_or_else(|| {
                "Dock context menu was not found after opening the Dock item menu.".to_string()
            })
    }

    fn launch_dock_item(item: &MacControlDockItem) -> Result<(), String> {
        let workspace = NSWorkspace::sharedWorkspace();
        if let Some(bundle_id) = item.bundle_id.as_deref() {
            let request = MacControlAppsRequest {
                op: MacControlAppsOp::Launch,
                bundle_id: Some(bundle_id.to_string()),
                limit: 1,
                ..Default::default()
            };
            let app = launch_app(&request)?;
            activate_running_app(&app)?;
            return Ok(());
        }
        let path = item
            .path
            .as_deref()
            .ok_or_else(|| "Dock item has no bundleId or path to launch.".to_string())?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        if workspace.openURL(&url) {
            Ok(())
        } else {
            Err(format!("macOS refused to open Dock item path '{path}'."))
        }
    }

    fn set_dock_autohide(value: bool) -> Result<(), String> {
        let bool_arg = if value { "true" } else { "false" };
        let output = Command::new("/usr/bin/defaults")
            .args(["write", "com.apple.dock", "autohide", "-bool", bool_arg])
            .output()
            .map_err(|e| format!("Failed to update Dock autohide preference: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "defaults write com.apple.dock autohide failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let kill_output = Command::new("/usr/bin/killall")
            .arg("Dock")
            .output()
            .map_err(|e| format!("Failed to restart Dock after autohide update: {e}"))?;
        if !kill_output.status.success() {
            return Err(format!(
                "killall Dock failed after autohide update: {}",
                String::from_utf8_lossy(&kill_output.stderr).trim()
            ));
        }
        Ok(())
    }

    fn read_spaces_state() -> Result<SpacesState, String> {
        match read_spaces_state_cgs() {
            Ok(state) => return Ok(state),
            Err(cgs_error) => {
                let mut state = read_spaces_state_defaults()?;
                state.warnings.insert(
                    0,
                    format!(
                        "Unable to read live CGS Spaces state; fell back to com.apple.spaces defaults: {cgs_error}"
                    ),
                );
                Ok(state)
            }
        }
    }

    fn read_spaces_state_cgs() -> Result<SpacesState, String> {
        type CGSDefaultConnectionFn = unsafe extern "C" fn() -> u32;
        type CGSCopyManagedDisplaySpacesFn = unsafe extern "C" fn(u32) -> CFArrayRef;
        type CGSGetActiveSpaceFn = unsafe extern "C" fn(u32) -> usize;

        let handle = load_private_framework(
            "/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight",
        )?;
        let connection_fn: CGSDefaultConnectionFn =
            unsafe { load_private_symbol(handle.0, "_CGSDefaultConnection")? };
        let copy_spaces: CGSCopyManagedDisplaySpacesFn =
            unsafe { load_private_symbol(handle.0, "CGSCopyManagedDisplaySpaces")? };
        let get_active_space: CGSGetActiveSpaceFn =
            unsafe { load_private_symbol(handle.0, "CGSGetActiveSpace")? };
        let connection = unsafe { connection_fn() };
        let active_space_id = match unsafe { get_active_space(connection) } {
            0 => None,
            id => Some(id as u64),
        };
        let spaces_ref = unsafe { copy_spaces(connection) };
        let spaces = CfOwned::new(spaces_ref as CFTypeRef)
            .ok_or_else(|| "CGSCopyManagedDisplaySpaces returned null.".to_string())?;
        let root = plist_from_cf_property_list(spaces.as_ptr())?;
        let mut displays = plist_array(&root)
            .ok_or_else(|| "CGSCopyManagedDisplaySpaces did not return an array.".to_string())?
            .iter()
            .filter_map(|value| spaces_display_from_plist(value, active_space_id))
            .collect::<Vec<_>>();
        let mut warnings = Vec::new();
        if displays.is_empty() {
            if let Some(display) =
                read_cgs_copy_spaces_display(handle.0, connection, active_space_id)?
            {
                displays.push(display);
            } else {
                warnings.push("No Spaces displays were found in live CGS state.".to_string());
            }
        } else if let Some(active_space_id) = active_space_id {
            let active_missing = !displays.iter().any(|display| {
                display
                    .spaces
                    .iter()
                    .any(|space| space.id == Some(active_space_id))
            });
            if active_missing {
                match read_cgs_copy_spaces_display(handle.0, connection, Some(active_space_id)) {
                    Ok(Some(display)) => {
                        warnings.push(format!(
                            "CGSGetActiveSpace returned Space id {active_space_id}, but it was not present in CGSCopyManagedDisplaySpaces; using CGSCopySpaces ordering instead."
                        ));
                        displays = vec![display];
                    }
                    Ok(None) => warnings.push(format!(
                        "CGSGetActiveSpace returned Space id {active_space_id}, but it was not present in CGSCopyManagedDisplaySpaces."
                    )),
                    Err(error) => warnings.push(format!(
                        "CGSGetActiveSpace returned Space id {active_space_id}, but it was not present in CGSCopyManagedDisplaySpaces and CGSCopySpaces fallback failed: {error}"
                    )),
                }
            }
        }
        Ok(SpacesState { displays, warnings })
    }

    fn read_cgs_copy_spaces_display(
        handle: *mut c_void,
        connection: u32,
        active_space_id: Option<u64>,
    ) -> Result<Option<MacControlSpacesDisplay>, String> {
        type CGSCopySpacesFn = unsafe extern "C" fn(u32, usize) -> CFArrayRef;

        let copy_spaces: CGSCopySpacesFn = unsafe { load_private_symbol(handle, "CGSCopySpaces")? };
        let spaces_ref = unsafe { copy_spaces(connection, K_CGS_ALL_SPACES_MASK) };
        let spaces = CfOwned::new(spaces_ref as CFTypeRef)
            .ok_or_else(|| "CGSCopySpaces returned null.".to_string())?;
        let root = plist_from_cf_property_list(spaces.as_ptr())?;
        let spaces = plist_array(&root)
            .map(|items| {
                items
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, item)| {
                        cgs_space_summary_from_value(item, idx + 1, active_space_id)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if spaces.is_empty() {
            return Ok(None);
        }
        let current_space = spaces.iter().find(|space| space.current).cloned();
        Ok(Some(MacControlSpacesDisplay {
            display_identifier: Some("kCGSPackagesMainDisplayIdentifier".to_string()),
            current_space,
            spaces,
            collapsed_space: None,
        }))
    }

    fn read_spaces_state_defaults() -> Result<SpacesState, String> {
        let root = read_defaults_domain("com.apple.spaces")?;
        let monitors = plist_get(&root, "SpacesDisplayConfiguration")
            .and_then(plist_dict)
            .and_then(|config| config.get("Management Data"))
            .and_then(plist_dict)
            .and_then(|management| management.get("Monitors"))
            .and_then(plist_array)
            .ok_or_else(|| {
                "Unable to read Mission Control Spaces from com.apple.spaces.".to_string()
            })?;
        let displays = monitors
            .iter()
            .filter_map(|value| spaces_display_from_plist(value, None))
            .collect::<Vec<_>>();
        let warnings = if displays.is_empty() {
            vec!["No Spaces displays were found in com.apple.spaces.".to_string()]
        } else {
            Vec::new()
        };
        Ok(SpacesState { displays, warnings })
    }

    fn plist_from_cf_property_list(value: CFTypeRef) -> Result<PlistValue, String> {
        let mut error: CFTypeRef = ptr::null();
        let data = unsafe {
            CFPropertyListCreateData(
                ptr::null(),
                value,
                K_CF_PROPERTY_LIST_XML_FORMAT_V1_0,
                0,
                &mut error,
            )
        };
        if data.is_null() {
            let detail = if error.is_null() {
                "unknown CoreFoundation error".to_string()
            } else {
                let error = CfOwned::new(error).expect("non-null CFError");
                let description = unsafe { CFErrorCopyDescription(error.as_ptr()) };
                CfOwned::new(description as CFTypeRef)
                    .and_then(|description| cf_value_string(description.as_ptr()))
                    .unwrap_or_else(|| "unknown CoreFoundation error".to_string())
            };
            return Err(format!("CFPropertyListCreateData failed: {detail}"));
        }

        let data = CfOwned::new(data as CFTypeRef)
            .ok_or_else(|| "CFPropertyListCreateData returned null.".to_string())?;
        let len = unsafe { CFDataGetLength(data.as_ptr() as CFDataRef) };
        if len < 0 {
            return Err("CFPropertyListCreateData returned a negative data length.".to_string());
        }
        let bytes = unsafe {
            let ptr = CFDataGetBytePtr(data.as_ptr() as CFDataRef);
            if ptr.is_null() {
                return Err("CFDataGetBytePtr returned null.".to_string());
            }
            std::slice::from_raw_parts(ptr, len as usize)
        };
        let xml = std::str::from_utf8(bytes)
            .map_err(|error| format!("CGS Spaces property list was not UTF-8 XML: {error}"))?;
        parse_plist_xml(xml)
    }

    fn spaces_display_from_plist(
        value: &PlistValue,
        active_space_id: Option<u64>,
    ) -> Option<MacControlSpacesDisplay> {
        let dict = plist_dict(value)?;
        let display_identifier = dict
            .get("Display Identifier")
            .and_then(plist_string)
            .map(ToString::to_string);
        let current_id = dict
            .get("Current Space")
            .and_then(space_summary_id_from_value)
            .or(active_space_id);
        let spaces = dict
            .get("Spaces")
            .and_then(plist_array)
            .map(|items| {
                items
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, item)| space_summary_from_value(item, idx + 1, current_id))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let current_space = spaces
            .iter()
            .find(|space| space.current)
            .cloned()
            .or_else(|| {
                dict.get("Current Space")
                    .and_then(|value| space_summary_from_value(value, 1, current_id))
            });
        let collapsed_space = dict
            .get("Collapsed Space")
            .and_then(|value| space_summary_from_value(value, 1, current_id));
        Some(MacControlSpacesDisplay {
            display_identifier,
            current_space,
            spaces,
            collapsed_space,
        })
    }

    fn space_summary_from_value(
        value: &PlistValue,
        index: usize,
        current_id: Option<u64>,
    ) -> Option<MacControlSpaceSummary> {
        let dict = plist_dict(value)?;
        let id = space_summary_id(dict);
        Some(MacControlSpaceSummary {
            id,
            uuid: dict
                .get("uuid")
                .and_then(plist_string)
                .map(ToString::to_string)
                .filter(|value| !value.is_empty()),
            index,
            kind: dict
                .get("type")
                .and_then(plist_integer)
                .map(|kind| kind.to_string()),
            current: id.is_some() && id == current_id,
        })
    }

    fn cgs_space_summary_from_value(
        value: &PlistValue,
        index: usize,
        current_id: Option<u64>,
    ) -> Option<MacControlSpaceSummary> {
        if let PlistValue::Integer(id) = value {
            let id = u64::try_from(*id).ok();
            return Some(MacControlSpaceSummary {
                id,
                uuid: None,
                index,
                kind: None,
                current: id.is_some() && id == current_id,
            });
        }
        space_summary_from_value(value, index, current_id)
    }

    fn space_summary_id_from_value(value: &PlistValue) -> Option<u64> {
        plist_dict(value).and_then(space_summary_id)
    }

    fn space_summary_id(dict: &BTreeMap<String, PlistValue>) -> Option<u64> {
        dict.get("ManagedSpaceID")
            .or_else(|| dict.get("id64"))
            .or_else(|| dict.get("wsid"))
            .or_else(|| dict.get("id"))
            .and_then(plist_integer)
            .and_then(|value| u64::try_from(value).ok())
    }

    fn resolve_spaces_switch_target(
        request: &MacControlSpacesRequest,
        displays: &[MacControlSpacesDisplay],
    ) -> Result<SpacesSwitchTarget, String> {
        if let Some(space_id) = request.space_id {
            return displays
                .iter()
                .find_map(|display| {
                    display
                        .spaces
                        .iter()
                        .find(|space| space.id == Some(space_id))
                        .cloned()
                        .map(|space| SpacesSwitchTarget { space })
                })
                .ok_or_else(|| {
                    format!("Space id {space_id} was not found in spaces.list output.")
                });
        }

        if let Some(index) = request.space_index {
            let preferred = current_spaces_display(displays)
                .or_else(|| displays.iter().find(|display| !display.spaces.is_empty()));
            if let Some(display) = preferred {
                if let Some(space) = display
                    .spaces
                    .iter()
                    .find(|space| space.index == index)
                    .cloned()
                {
                    return Ok(SpacesSwitchTarget { space });
                }
            }
            return displays
                .iter()
                .find_map(|display| {
                    display
                        .spaces
                        .iter()
                        .find(|space| space.index == index)
                        .cloned()
                        .map(|space| SpacesSwitchTarget { space })
                })
                .ok_or_else(|| {
                    format!("Space index {index} was not found in spaces.list output.")
                });
        }

        let Some(direction) = request.direction else {
            return Err("spaces.switch requires spaceId, spaceIndex, or direction.".to_string());
        };
        let display = current_spaces_display(displays)
            .ok_or_else(|| "Unable to resolve the current Spaces display.".to_string())?;
        let current_id = display.current_space.as_ref().and_then(|space| space.id);
        let current_pos = display
            .spaces
            .iter()
            .position(|space| space.current || (current_id.is_some() && space.id == current_id))
            .ok_or_else(|| {
                "Unable to resolve the current Space within the current display.".to_string()
            })?;
        let target_pos = match direction {
            MacControlSpaceDirection::Left => current_pos.checked_sub(1),
            MacControlSpaceDirection::Right => current_pos.checked_add(1),
        }
        .filter(|index| *index < display.spaces.len())
        .ok_or_else(|| {
            let label = match direction {
                MacControlSpaceDirection::Left => "left",
                MacControlSpaceDirection::Right => "right",
            };
            format!("No Space exists to the {label} of the current Space.")
        })?;

        Ok(SpacesSwitchTarget {
            space: display.spaces[target_pos].clone(),
        })
    }

    fn current_spaces_display(
        displays: &[MacControlSpacesDisplay],
    ) -> Option<&MacControlSpacesDisplay> {
        displays
            .iter()
            .find(|display| {
                display
                    .current_space
                    .as_ref()
                    .is_some_and(|space| space.current)
                    || display.spaces.iter().any(|space| space.current)
            })
            .or_else(|| {
                displays.iter().find(|display| {
                    display
                        .current_space
                        .as_ref()
                        .is_some_and(|space| space.id.is_some())
                })
            })
    }

    fn preferred_spaces_switch_hotkey(
        request: &MacControlSpacesRequest,
        displays: &[MacControlSpacesDisplay],
        target: &MacControlSpaceSummary,
    ) -> Option<(Vec<String>, String)> {
        if request.direction.is_some() {
            return spaces_switch_hotkey(request, displays)
                .ok()
                .map(|(keys, label)| {
                    (
                        keys,
                        label.replacen("CGEventHotkey", "MissionControlHotkey", 1),
                    )
                });
        }

        let display = current_spaces_display(displays)?;
        let current_id = display.current_space.as_ref().and_then(|space| space.id);
        let current_pos = display
            .spaces
            .iter()
            .position(|space| space.current || (current_id.is_some() && space.id == current_id))?;
        let target_pos = display
            .spaces
            .iter()
            .position(|space| space_matches_summary(Some(space), target))?;

        if target_pos == current_pos {
            if let Some(index) = request.space_index.filter(|index| (1..=9).contains(index)) {
                return Some((
                    vec!["ctrl".to_string(), index.to_string()],
                    format!("MissionControlHotkey ctrl+{index}"),
                ));
            }
            return Some((Vec::new(), "NoOp already on requested Space".to_string()));
        }
        if target_pos + 1 == current_pos {
            return Some((
                vec!["ctrl".to_string(), "left".to_string()],
                "MissionControlHotkey ctrl+left".to_string(),
            ));
        }
        if target_pos == current_pos + 1 {
            return Some((
                vec!["ctrl".to_string(), "right".to_string()],
                "MissionControlHotkey ctrl+right".to_string(),
            ));
        }

        let index = request.space_index.unwrap_or(target.index);
        (1..=9).contains(&index).then(|| {
            (
                vec!["ctrl".to_string(), index.to_string()],
                format!("MissionControlHotkey ctrl+{index}"),
            )
        })
    }

    fn refresh_spaces_after_action(
        spaces: &mut SpacesState,
        switched: &mut Option<MacControlSpaceSummary>,
        warnings: &mut Vec<String>,
    ) -> bool {
        match read_spaces_state() {
            Ok(next) => {
                *switched = current_space_from_displays(&next.displays);
                spaces.displays = next.displays;
                warnings.extend(next.warnings);
                true
            }
            Err(error) => {
                warnings.push(format!("Unable to refresh Spaces after switch: {error}"));
                false
            }
        }
    }

    fn refresh_spaces_after_switch(
        spaces: &mut SpacesState,
        switched: &mut Option<MacControlSpaceSummary>,
        warnings: &mut Vec<String>,
        target: &MacControlSpaceSummary,
    ) -> bool {
        match read_spaces_state() {
            Ok(next) => {
                *switched = current_space_from_displays(&next.displays);
                spaces.displays = next.displays;
                warnings.extend(next.warnings);
                space_matches_summary(switched.as_ref(), target)
            }
            Err(error) => {
                warnings.push(format!("Unable to refresh Spaces after switch: {error}"));
                false
            }
        }
    }

    fn same_space_summary(
        left: Option<&MacControlSpaceSummary>,
        right: Option<&MacControlSpaceSummary>,
    ) -> bool {
        let (Some(left), Some(right)) = (left, right) else {
            return false;
        };
        space_matches_summary(Some(left), right)
    }

    fn space_matches_summary(
        actual: Option<&MacControlSpaceSummary>,
        expected: &MacControlSpaceSummary,
    ) -> bool {
        let Some(actual) = actual else {
            return false;
        };
        match (actual.id, expected.id) {
            (Some(actual_id), Some(expected_id)) => actual_id == expected_id,
            _ => actual.index == expected.index,
        }
    }

    fn post_mission_control_hotkey(
        keys: &[String],
        label: &str,
        warnings: &mut Vec<String>,
    ) -> Result<String, String> {
        let suffix = label
            .strip_prefix("MissionControlHotkey ")
            .or_else(|| label.strip_prefix("CGEventHotkey "))
            .unwrap_or(label);
        match post_system_events_hotkey(keys) {
            Ok(()) => Ok(format!("SystemEventsHotkey {suffix}")),
            Err(system_events_error) => {
                warnings.push(format!(
                    "System Events Mission Control hotkey failed; falling back to CGEvent: {system_events_error}"
                ));
                post_hotkey(keys).map_err(|cg_event_error| {
                    format!(
                        "System Events Mission Control hotkey failed ({system_events_error}); CGEvent fallback failed ({cg_event_error})"
                    )
                })?;
                Ok(format!("CGEventHotkey {suffix}"))
            }
        }
    }

    fn switch_visible_space_with_cgs(space_id: u64) -> Result<(), String> {
        type CGSDefaultConnectionFn = unsafe extern "C" fn() -> u32;
        type CGSManagedDisplaySetCurrentSpaceFn = unsafe extern "C" fn(u32, CFStringRef, usize);

        let handle = load_private_framework(
            "/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight",
        )?;
        let connection_fn: CGSDefaultConnectionFn =
            unsafe { load_private_symbol(handle.0, "_CGSDefaultConnection")? };
        let set_current_space: CGSManagedDisplaySetCurrentSpaceFn =
            unsafe { load_private_symbol(handle.0, "CGSManagedDisplaySetCurrentSpace")? };
        let display = unsafe {
            load_private_cfstring_constant(handle.0, "kCGSPackagesMainDisplayIdentifier")?
        };
        let connection = unsafe { connection_fn() };
        let space_id = usize::try_from(space_id)
            .map_err(|_| format!("Space id {space_id} does not fit in CGSSpaceID."))?;
        unsafe {
            set_current_space(connection, display, space_id);
        }
        Ok(())
    }

    fn cgs_spaces_for_window(window_id: u32) -> Result<Vec<u64>, String> {
        type CGSDefaultConnectionFn = unsafe extern "C" fn() -> u32;

        let handle = load_private_framework(
            "/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight",
        )?;
        let connection_fn: CGSDefaultConnectionFn =
            unsafe { load_private_symbol(handle.0, "_CGSDefaultConnection")? };
        let connection = unsafe { connection_fn() };
        let window_array = cf_number_array(&[i64::from(window_id)], "CGWindowID")?;
        cgs_spaces_for_window_array(handle.0, connection, window_array.as_ptr() as CFArrayRef)
    }

    fn move_window_to_space_with_cgs(
        window_id: u32,
        target_space_id: u64,
    ) -> Result<Vec<u64>, String> {
        type CGSDefaultConnectionFn = unsafe extern "C" fn() -> u32;
        type CGSRemoveWindowsFromSpacesFn = unsafe extern "C" fn(u32, CFArrayRef, CFArrayRef);
        type CGSAddWindowsToSpacesFn = unsafe extern "C" fn(u32, CFArrayRef, CFArrayRef);

        let handle = load_private_framework(
            "/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight",
        )?;
        let connection_fn: CGSDefaultConnectionFn =
            unsafe { load_private_symbol(handle.0, "_CGSDefaultConnection")? };
        let remove_windows: CGSRemoveWindowsFromSpacesFn =
            unsafe { load_private_symbol(handle.0, "CGSRemoveWindowsFromSpaces")? };
        let add_windows: CGSAddWindowsToSpacesFn =
            unsafe { load_private_symbol(handle.0, "CGSAddWindowsToSpaces")? };
        let connection = unsafe { connection_fn() };
        let window_array = cf_number_array(&[i64::from(window_id)], "CGWindowID")?;
        let target_space = i64::try_from(target_space_id)
            .map_err(|_| format!("Space id {target_space_id} does not fit in CFNumber."))?;
        let target_space_array = cf_number_array(&[target_space], "CGSSpaceID")?;
        let current_spaces =
            cgs_spaces_for_window_array(handle.0, connection, window_array.as_ptr() as CFArrayRef)?;
        if !current_spaces.is_empty() {
            let previous_space_values = current_spaces
                .iter()
                .map(|space_id| {
                    i64::try_from(*space_id)
                        .map_err(|_| format!("Space id {space_id} does not fit in CFNumber."))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let previous_space_array = cf_number_array(&previous_space_values, "CGSSpaceID")?;
            unsafe {
                remove_windows(
                    connection,
                    window_array.as_ptr() as CFArrayRef,
                    previous_space_array.as_ptr() as CFArrayRef,
                );
            }
        }
        unsafe {
            add_windows(
                connection,
                window_array.as_ptr() as CFArrayRef,
                target_space_array.as_ptr() as CFArrayRef,
            );
        }
        Ok(current_spaces)
    }

    fn cgs_spaces_for_window_array(
        handle: *mut c_void,
        connection: u32,
        window_array: CFArrayRef,
    ) -> Result<Vec<u64>, String> {
        type CGSCopySpacesForWindowsFn = unsafe extern "C" fn(u32, usize, CFArrayRef) -> CFArrayRef;

        let copy_spaces_for_windows: CGSCopySpacesForWindowsFn =
            unsafe { load_private_symbol(handle, "CGSCopySpacesForWindows")? };
        let spaces_ref =
            unsafe { copy_spaces_for_windows(connection, K_CGS_ALL_SPACES_MASK, window_array) };
        let spaces = CfOwned::new(spaces_ref as CFTypeRef)
            .ok_or_else(|| "CGSCopySpacesForWindows returned null.".to_string())?;
        cgs_space_ids_from_cf_array(spaces.as_ptr())
    }

    fn cgs_space_ids_from_cf_array(value: CFTypeRef) -> Result<Vec<u64>, String> {
        let root = plist_from_cf_property_list(value)?;
        let items = plist_array(&root)
            .ok_or_else(|| "CGSCopySpacesForWindows did not return an array.".to_string())?;
        Ok(items
            .iter()
            .filter_map(|item| match item {
                PlistValue::Integer(id) => u64::try_from(*id).ok(),
                PlistValue::Dict(dict) => space_summary_id(dict),
                _ => None,
            })
            .collect())
    }

    struct DlHandle(*mut c_void);

    impl Drop for DlHandle {
        fn drop(&mut self) {
            unsafe {
                libc::dlclose(self.0);
            }
        }
    }

    fn load_private_framework(path: &str) -> Result<DlHandle, String> {
        let path = CString::new(path).map_err(|e| format!("invalid framework path: {e}"))?;
        let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_LAZY) };
        if handle.is_null() {
            return Err(format!("dlopen failed: {}", dl_error_message()));
        }
        Ok(DlHandle(handle as *mut c_void))
    }

    unsafe fn load_private_symbol<T>(handle: *mut c_void, symbol: &str) -> Result<T, String>
    where
        T: Copy,
    {
        let symbol = CString::new(symbol).map_err(|e| format!("invalid symbol name: {e}"))?;
        let ptr = unsafe { libc::dlsym(handle as *mut libc::c_void, symbol.as_ptr()) };
        if ptr.is_null() {
            return Err(format!(
                "dlsym({}) failed: {}",
                symbol.to_string_lossy(),
                dl_error_message()
            ));
        }
        Ok(unsafe { std::mem::transmute_copy(&ptr) })
    }

    unsafe fn load_private_cfstring_constant(
        handle: *mut c_void,
        symbol: &str,
    ) -> Result<CFStringRef, String> {
        let symbol = CString::new(symbol).map_err(|e| format!("invalid symbol name: {e}"))?;
        let ptr = unsafe { libc::dlsym(handle as *mut libc::c_void, symbol.as_ptr()) };
        if ptr.is_null() {
            return Err(format!(
                "dlsym({}) failed: {}",
                symbol.to_string_lossy(),
                dl_error_message()
            ));
        }
        let value = unsafe { *(ptr as *const CFStringRef) };
        if value.is_null() {
            return Err(format!(
                "dlsym({}) returned a null CFString constant.",
                symbol.to_string_lossy()
            ));
        }
        let type_id = unsafe { CFGetTypeID(value as CFTypeRef) };
        if type_id != unsafe { CFStringGetTypeID() } {
            return Err(format!(
                "dlsym({}) did not resolve to a CFString constant.",
                symbol.to_string_lossy()
            ));
        }
        Ok(value)
    }

    fn dl_error_message() -> String {
        let error = unsafe { libc::dlerror() };
        if error.is_null() {
            "unknown dynamic loader error".to_string()
        } else {
            unsafe { CStr::from_ptr(error) }
                .to_string_lossy()
                .into_owned()
        }
    }

    fn spaces_switch_hotkey(
        request: &MacControlSpacesRequest,
        displays: &[MacControlSpacesDisplay],
    ) -> Result<(Vec<String>, String), String> {
        if let Some(direction) = request.direction {
            let key = match direction {
                MacControlSpaceDirection::Left => "left",
                MacControlSpaceDirection::Right => "right",
            };
            return Ok((
                vec!["ctrl".to_string(), key.to_string()],
                format!("CGEventHotkey ctrl+{key}"),
            ));
        }

        let index = if let Some(index) = request.space_index {
            index
        } else if let Some(space_id) = request.space_id {
            displays
                .iter()
                .flat_map(|display| display.spaces.iter())
                .find(|space| space.id == Some(space_id))
                .map(|space| space.index)
                .ok_or_else(|| {
                    format!("Space id {space_id} was not found in spaces.list output.")
                })?
        } else {
            return Err("spaces.switch requires spaceId, spaceIndex, or direction.".to_string());
        };

        if !(1..=9).contains(&index) {
            return Err(
                "spaces.switch by index only supports 1..=9 because macOS exposes Control+number shortcuts for those Spaces."
                    .to_string(),
            );
        }
        Ok((
            vec!["ctrl".to_string(), index.to_string()],
            format!("CGEventHotkey ctrl+{index}"),
        ))
    }

    fn current_space_from_displays(
        displays: &[MacControlSpacesDisplay],
    ) -> Option<MacControlSpaceSummary> {
        displays
            .iter()
            .find_map(|display| {
                display
                    .current_space
                    .as_ref()
                    .filter(|space| space.current)
                    .cloned()
            })
            .or_else(|| {
                displays
                    .iter()
                    .flat_map(|display| display.spaces.iter())
                    .find(|space| space.current)
                    .cloned()
            })
            .or_else(|| {
                displays
                    .iter()
                    .find_map(|display| display.current_space.clone())
            })
    }

    fn read_defaults_domain(domain: &str) -> Result<PlistValue, String> {
        let output = Command::new("/usr/bin/defaults")
            .args(["export", domain, "-"])
            .output()
            .map_err(|e| format!("Failed to read defaults domain '{domain}': {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "defaults export {domain} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let xml = String::from_utf8(output.stdout)
            .map_err(|e| format!("defaults export {domain} returned non-UTF-8 XML: {e}"))?;
        parse_plist_xml(&xml)
    }

    fn parse_plist_xml(xml: &str) -> Result<PlistValue, String> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) if e.local_name().as_ref() == b"plist" => {
                    return read_plist_child(&mut reader, b"plist");
                }
                Ok(Event::Eof) => return Err("plist XML ended before <plist>.".to_string()),
                Ok(_) => {}
                Err(e) => return Err(format!("Unable to parse plist XML: {e}")),
            }
            buf.clear();
        }
    }

    fn read_plist_child(reader: &mut Reader<&[u8]>, end: &[u8]) -> Result<PlistValue, String> {
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => return read_plist_value(reader, e),
                Ok(Event::Empty(e)) => return read_empty_plist_value(e),
                Ok(Event::End(e)) if e.local_name().as_ref() == end => {
                    return Err(format!(
                        "plist XML <{}> did not contain a value.",
                        String::from_utf8_lossy(end)
                    ));
                }
                Ok(Event::Eof) => return Err("plist XML ended unexpectedly.".to_string()),
                Ok(_) => {}
                Err(e) => return Err(format!("Unable to parse plist XML: {e}")),
            }
            buf.clear();
        }
    }

    fn read_plist_value(
        reader: &mut Reader<&[u8]>,
        start: BytesStart<'_>,
    ) -> Result<PlistValue, String> {
        match start.local_name().as_ref() {
            b"dict" => read_plist_dict(reader),
            b"array" => read_plist_array(reader),
            b"string" | b"key" => {
                read_plist_text(reader, start.local_name().as_ref()).map(PlistValue::String)
            }
            b"integer" => read_plist_text(reader, b"integer")
                .map(|text| text.parse::<i64>().unwrap_or_default())
                .map(PlistValue::Integer),
            b"real" => read_plist_text(reader, b"real").map(|_| PlistValue::Real),
            b"data" => {
                let _ = read_plist_text(reader, b"data")?;
                Ok(PlistValue::Data)
            }
            b"true" => {
                consume_plist_end(reader, b"true")?;
                Ok(PlistValue::Bool(true))
            }
            b"false" => {
                consume_plist_end(reader, b"false")?;
                Ok(PlistValue::Bool(false))
            }
            other => Err(format!(
                "Unsupported plist element <{}>.",
                String::from_utf8_lossy(other)
            )),
        }
    }

    fn read_empty_plist_value(start: BytesStart<'_>) -> Result<PlistValue, String> {
        match start.local_name().as_ref() {
            b"dict" => Ok(PlistValue::Dict(BTreeMap::new())),
            b"array" => Ok(PlistValue::Array(Vec::new())),
            b"string" | b"key" => Ok(PlistValue::String(String::new())),
            b"true" => Ok(PlistValue::Bool(true)),
            b"false" => Ok(PlistValue::Bool(false)),
            b"data" => Ok(PlistValue::Data),
            other => Err(format!(
                "Unsupported empty plist element <{}>.",
                String::from_utf8_lossy(other)
            )),
        }
    }

    fn read_plist_dict(reader: &mut Reader<&[u8]>) -> Result<PlistValue, String> {
        let mut buf = Vec::new();
        let mut map = BTreeMap::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) if e.local_name().as_ref() == b"key" => {
                    let key = read_plist_text(reader, b"key")?;
                    let value = read_plist_child(reader, b"dict")?;
                    map.insert(key, value);
                }
                Ok(Event::End(e)) if e.local_name().as_ref() == b"dict" => {
                    return Ok(PlistValue::Dict(map));
                }
                Ok(Event::Eof) => return Err("plist dict ended unexpectedly.".to_string()),
                Ok(_) => {}
                Err(e) => return Err(format!("Unable to parse plist dict: {e}")),
            }
            buf.clear();
        }
    }

    fn read_plist_array(reader: &mut Reader<&[u8]>) -> Result<PlistValue, String> {
        let mut buf = Vec::new();
        let mut items = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => items.push(read_plist_value(reader, e)?),
                Ok(Event::Empty(e)) => items.push(read_empty_plist_value(e)?),
                Ok(Event::End(e)) if e.local_name().as_ref() == b"array" => {
                    return Ok(PlistValue::Array(items));
                }
                Ok(Event::Eof) => return Err("plist array ended unexpectedly.".to_string()),
                Ok(_) => {}
                Err(e) => return Err(format!("Unable to parse plist array: {e}")),
            }
            buf.clear();
        }
    }

    fn read_plist_text(reader: &mut Reader<&[u8]>, end: &[u8]) -> Result<String, String> {
        let mut buf = Vec::new();
        let mut text = String::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Text(e)) => {
                    // quick-xml 0.39：decode()（字符编码）+ escape::unescape()（XML
                    // 实体）组合等价于旧的 BytesText::unescape()。
                    let decoded = e
                        .decode()
                        .map_err(|e| format!("Unable to decode plist text: {e}"))?;
                    let unescaped = quick_xml::escape::unescape(&decoded)
                        .map_err(|e| format!("Unable to unescape plist text: {e}"))?;
                    text.push_str(&unescaped);
                }
                Ok(Event::End(e)) if e.local_name().as_ref() == end => return Ok(text),
                Ok(Event::Eof) => return Err("plist text ended unexpectedly.".to_string()),
                Ok(_) => {}
                Err(e) => return Err(format!("Unable to parse plist text: {e}")),
            }
            buf.clear();
        }
    }

    fn consume_plist_end(reader: &mut Reader<&[u8]>, end: &[u8]) -> Result<(), String> {
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::End(e)) if e.local_name().as_ref() == end => return Ok(()),
                Ok(Event::Eof) => return Err("plist XML ended unexpectedly.".to_string()),
                Ok(_) => {}
                Err(e) => return Err(format!("Unable to parse plist XML: {e}")),
            }
            buf.clear();
        }
    }

    fn plist_get<'a>(value: &'a PlistValue, key: &str) -> Option<&'a PlistValue> {
        plist_dict(value).and_then(|dict| dict.get(key))
    }

    fn plist_dict(value: &PlistValue) -> Option<&BTreeMap<String, PlistValue>> {
        match value {
            PlistValue::Dict(value) => Some(value),
            _ => None,
        }
    }

    fn plist_array(value: &PlistValue) -> Option<&[PlistValue]> {
        match value {
            PlistValue::Array(value) => Some(value),
            _ => None,
        }
    }

    fn plist_string(value: &PlistValue) -> Option<&str> {
        match value {
            PlistValue::String(value) => Some(value),
            _ => None,
        }
    }

    fn plist_integer(value: &PlistValue) -> Option<i64> {
        match value {
            PlistValue::Integer(value) => Some(*value),
            _ => None,
        }
    }

    fn plist_bool(value: &PlistValue) -> Option<bool> {
        match value {
            PlistValue::Bool(value) => Some(*value),
            _ => None,
        }
    }

    fn dock_url_to_path(value: &str) -> Option<String> {
        if let Ok(url) = url::Url::parse(value) {
            if url.scheme() == "file" {
                return url
                    .to_file_path()
                    .ok()
                    .map(|path| path.display().to_string());
            }
        }
        Some(value.to_string()).filter(|value| !value.is_empty())
    }

    fn handle_windows(
        request: MacControlWindowsRequest,
    ) -> Result<MacControlWindowsResult, String> {
        let request = request.clamped();
        let frontmost_app = focused_app_summary();
        let mut windows = list_windows_for_request(&request)?;
        let mut execution = None;
        let mut verification = None;
        let acted_window = if request.op == MacControlWindowsOp::List {
            None
        } else {
            let (window, summary) = resolve_window(&request)?;
            ensure_external_window_mutation(&summary, request.op)?;
            match request.op {
                MacControlWindowsOp::Focus => {
                    perform_ax_action(window.as_ptr() as AXUIElementRef, "AXRaise")?;
                    let _ = set_ax_bool(window.as_ptr() as AXUIElementRef, "AXMain", true);
                    let _ = set_ax_bool(window.as_ptr() as AXUIElementRef, "AXFocused", true);
                }
                MacControlWindowsOp::Move => {
                    let x = request
                        .x
                        .ok_or_else(|| "windows.move requires x.".to_string())?;
                    let y = request
                        .y
                        .ok_or_else(|| "windows.move requires y.".to_string())?;
                    set_ax_point(
                        window.as_ptr() as AXUIElementRef,
                        "AXPosition",
                        CGPoint { x, y },
                    )?;
                }
                MacControlWindowsOp::Resize => {
                    let width = request
                        .width
                        .ok_or_else(|| "windows.resize requires width.".to_string())?;
                    let height = request
                        .height
                        .ok_or_else(|| "windows.resize requires height.".to_string())?;
                    set_ax_size(
                        window.as_ptr() as AXUIElementRef,
                        "AXSize",
                        CGSize { width, height },
                    )?;
                }
                MacControlWindowsOp::Minimize => {
                    set_ax_bool(window.as_ptr() as AXUIElementRef, "AXMinimized", true)?;
                }
                MacControlWindowsOp::Close => {
                    let snapshot = capture_ax_snapshot(MacControlSnapshotRequest {
                        include_screenshot: false,
                        max_elements: request.max_elements,
                        max_depth: request.max_depth,
                        ..Default::default()
                    })?;
                    execution = Some(close_window(
                        window.as_ptr() as AXUIElementRef,
                        &summary,
                        &snapshot,
                    )?);
                }
                MacControlWindowsOp::List => {}
            }
            thread::sleep(Duration::from_millis(VERIFY_SETTLE_MS));
            let acted = window_summary(window.as_ptr() as AXUIElementRef, &summary.id);
            verification = Some(verify_window_action(&request, &summary, &acted));
            Some(acted)
        };
        if let Some(acted) = acted_window.clone() {
            if let Some(existing) = windows.iter_mut().find(|window| window.id == acted.id) {
                *existing = acted.clone();
            } else {
                windows.insert(0, acted.clone());
            }
        }
        Ok(MacControlWindowsResult {
            op: request.op,
            window_scope: request.window_scope,
            frontmost_app,
            windows,
            acted_window,
            execution,
            verification,
        })
    }

    fn verify_window_action(
        request: &MacControlWindowsRequest,
        before: &MacControlWindowSummary,
        after: &MacControlWindowSummary,
    ) -> MacControlVerification {
        match request.op {
            MacControlWindowsOp::Focus => verification_from_checks(
                "windows.focus verification",
                vec![verification_check(
                    "focused",
                    Some("true".to_string()),
                    Some(after.focused.to_string()),
                    after.focused,
                )],
                Vec::new(),
            ),
            MacControlWindowsOp::Move => {
                let expected_x = request.x;
                let expected_y = request.y;
                let actual = after.bounds_points;
                verification_from_checks(
                    "windows.move verification",
                    vec![
                        verification_check(
                            "x",
                            expected_x.map(format_number),
                            actual.map(|bounds| format_number(bounds.x)),
                            expected_x
                                .zip(actual.map(|bounds| bounds.x))
                                .is_some_and(|(expected, actual)| numbers_close(expected, actual)),
                        ),
                        verification_check(
                            "y",
                            expected_y.map(format_number),
                            actual.map(|bounds| format_number(bounds.y)),
                            expected_y
                                .zip(actual.map(|bounds| bounds.y))
                                .is_some_and(|(expected, actual)| numbers_close(expected, actual)),
                        ),
                    ],
                    Vec::new(),
                )
            }
            MacControlWindowsOp::Resize => {
                let expected_width = request.width;
                let expected_height = request.height;
                let actual = after.bounds_points;
                verification_from_checks(
                    "windows.resize verification",
                    vec![
                        verification_check(
                            "width",
                            expected_width.map(format_number),
                            actual.map(|bounds| format_number(bounds.width)),
                            expected_width
                                .zip(actual.map(|bounds| bounds.width))
                                .is_some_and(|(expected, actual)| numbers_close(expected, actual)),
                        ),
                        verification_check(
                            "height",
                            expected_height.map(format_number),
                            actual.map(|bounds| format_number(bounds.height)),
                            expected_height
                                .zip(actual.map(|bounds| bounds.height))
                                .is_some_and(|(expected, actual)| numbers_close(expected, actual)),
                        ),
                    ],
                    Vec::new(),
                )
            }
            MacControlWindowsOp::Close => verify_window_closed(before),
            MacControlWindowsOp::Minimize => MacControlVerification {
                status: MacControlVerificationStatus::Unverified,
                summary: "windows.minimize verification unavailable because AX summary does not expose minimized state.".to_string(),
                checks: Vec::new(),
                warnings: vec![
                    "Use windows.list or a fresh snapshot if the next step depends on minimized state."
                        .to_string(),
                ],
            },
            MacControlWindowsOp::List => MacControlVerification {
                status: MacControlVerificationStatus::Unverified,
                summary: "windows.list is read-only and has no mutation to verify.".to_string(),
                checks: Vec::new(),
                warnings: Vec::new(),
            },
        }
    }

    fn verify_window_closed(before: &MacControlWindowSummary) -> MacControlVerification {
        match window_still_present(before) {
            Ok(false) => verification_from_checks(
                "windows.close verification",
                vec![verification_check(
                    "windowGone",
                    Some("true".to_string()),
                    Some("true".to_string()),
                    true,
                )],
                Vec::new(),
            ),
            Ok(true) => verification_from_checks(
                "windows.close verification",
                vec![verification_check(
                    "windowGone",
                    Some("true".to_string()),
                    Some("false".to_string()),
                    false,
                )],
                Vec::new(),
            ),
            Err(error) => MacControlVerification {
                status: MacControlVerificationStatus::Unverified,
                summary: "windows.close verification could not inspect windows after close."
                    .to_string(),
                checks: Vec::new(),
                warnings: vec![error],
            },
        }
    }

    fn window_still_present(before: &MacControlWindowSummary) -> Result<bool, String> {
        let Some(pid) = before.app_pid else {
            return Err(
                "windows.close verification requires the original window app pid.".to_string(),
            );
        };
        let app = app_element_for_pid(pid).ok_or_else(|| {
            format!("windows.close verification could not resolve app pid {pid}.")
        })?;
        let Some(windows) = copy_attribute(app.as_ptr() as AXUIElementRef, "AXWindows") else {
            return Ok(false);
        };
        Ok(cf_array_values(windows.as_ptr())
            .into_iter()
            .enumerate()
            .map(|(idx, window_ref)| {
                window_summary_for_app(
                    window_ref as AXUIElementRef,
                    &format!("win_{idx}"),
                    Some(pid),
                )
            })
            .any(|candidate| window_fingerprint_matches(&candidate, before)))
    }

    fn window_fingerprint_matches(
        candidate: &MacControlWindowSummary,
        expected: &MacControlWindowSummary,
    ) -> bool {
        if candidate.role != expected.role || candidate.subrole != expected.subrole {
            return false;
        }
        if candidate.title != expected.title {
            return false;
        }
        match (candidate.bounds_points, expected.bounds_points) {
            (Some(candidate), Some(expected)) => bounds_match(Some(candidate), Some(expected)),
            (None, None) => true,
            _ => false,
        }
    }

    fn verification_from_checks(
        summary: &str,
        checks: Vec<MacControlVerificationCheck>,
        warnings: Vec<String>,
    ) -> MacControlVerification {
        let status = if checks.is_empty() {
            MacControlVerificationStatus::Unverified
        } else if checks.iter().all(|check| check.passed) {
            MacControlVerificationStatus::Verified
        } else {
            MacControlVerificationStatus::Failed
        };
        MacControlVerification {
            status,
            summary: match status {
                MacControlVerificationStatus::Verified => format!("{summary}: verified"),
                MacControlVerificationStatus::Failed => format!("{summary}: failed"),
                MacControlVerificationStatus::Unverified => format!("{summary}: unverified"),
            },
            checks,
            warnings,
        }
    }

    fn verification_check(
        name: &str,
        expected: Option<String>,
        actual: Option<String>,
        passed: bool,
    ) -> MacControlVerificationCheck {
        MacControlVerificationCheck {
            name: name.to_string(),
            expected,
            actual,
            passed,
        }
    }

    fn numbers_close(expected: f64, actual: f64) -> bool {
        (expected - actual).abs() <= 4.0
    }

    fn format_number(value: f64) -> String {
        if value.fract().abs() < f64::EPSILON {
            format!("{value:.0}")
        } else {
            format!("{value:.2}")
        }
    }

    enum AxValueVerificationMode {
        Exact,
        Append { before: Option<String> },
    }

    fn verify_ax_value(
        element: AXUIElementRef,
        expected: &str,
        label: &str,
        mode: AxValueVerificationMode,
    ) -> MacControlVerification {
        thread::sleep(Duration::from_millis(VERIFY_SETTLE_MS));
        let actual = attribute_string(element, "AXValue");
        let (check_name, passed, warnings) = match mode {
            AxValueVerificationMode::Exact => (
                "valueEquals",
                actual.as_deref().is_some_and(|actual| actual == expected),
                Vec::new(),
            ),
            AxValueVerificationMode::Append { before } => {
                let Some(before) = before else {
                    return MacControlVerification {
                        status: MacControlVerificationStatus::Unverified,
                        summary: format!("{label} value verification: unverified"),
                        checks: vec![verification_check(
                            "valueChangedAndContains",
                            Some(expected.to_string()),
                            actual,
                            false,
                        )],
                        warnings: vec![
                            "AXValue before input was unavailable, so append verification cannot prove the value changed.".to_string(),
                        ],
                    };
                };
                let passed = actual
                    .as_deref()
                    .is_some_and(|actual| actual != before && actual.contains(expected));
                ("valueChangedAndContains", passed, Vec::new())
            }
        };
        verification_from_checks(
            &format!("{label} value verification"),
            vec![verification_check(
                check_name,
                Some(expected.to_string()),
                actual,
                passed,
            )],
            warnings,
        )
    }

    fn verify_pointer_position(expected: CGPoint, label: &str) -> MacControlVerification {
        thread::sleep(Duration::from_millis(VERIFY_SETTLE_MS));
        match current_mouse_position() {
            Ok(actual) => verification_from_checks(
                &format!("{label} pointer verification"),
                vec![
                    verification_check(
                        "pointerX",
                        Some(format_number(expected.x)),
                        Some(format_number(actual.x)),
                        numbers_close(expected.x, actual.x),
                    ),
                    verification_check(
                        "pointerY",
                        Some(format_number(expected.y)),
                        Some(format_number(actual.y)),
                        numbers_close(expected.y, actual.y),
                    ),
                ],
                Vec::new(),
            ),
            Err(error) => MacControlVerification {
                status: MacControlVerificationStatus::Unverified,
                summary: format!("{label} pointer verification: unverified"),
                checks: Vec::new(),
                warnings: vec![error],
            },
        }
    }

    fn handle_elements(
        request: MacControlElementsRequest,
    ) -> Result<MacControlElementsResult, String> {
        let request = request.clamped();
        let snapshot = capture_ax_snapshot(MacControlSnapshotRequest {
            include_screenshot: false,
            max_elements: request.max_elements,
            max_depth: request.max_depth,
            ..Default::default()
        })?;
        let mut warnings = snapshot.warnings.clone();
        ha_core::mac_control::record_snapshot(snapshot.clone());
        let (total_matches, elements) = if frontmost_app_matches_act_target(
            &snapshot,
            &request.target,
        ) {
            let mut candidates = snapshot
                .elements
                .iter()
                .filter(|element| element_matches_query(element, &request.target, &snapshot))
                .map(|element| element_candidate(element, &request.target, &snapshot))
                .collect::<Vec<_>>();
            candidates.sort_by(|left, right| {
                right
                    .score
                    .cmp(&left.score)
                    .then_with(|| left.element.id.cmp(&right.element.id))
            });
            let total_matches = candidates.len();
            if total_matches > request.limit {
                warnings.push(format!(
                    "elements.find matched {total_matches} candidates; returning top {}.",
                    request.limit
                ));
            }
            candidates.truncate(request.limit);
            (total_matches, candidates)
        } else {
            warnings.push(
                "Frontmost app did not match the elements.find target; activate the target app first."
                    .to_string(),
            );
            (0, Vec::new())
        };

        Ok(MacControlElementsResult {
            op: request.op,
            target: request.target,
            snapshot_id: snapshot.snapshot_id,
            created_at: snapshot.created_at,
            frontmost_app: snapshot.frontmost_app,
            total_matches,
            elements,
            truncated: snapshot.truncated,
            warnings,
        })
    }

    fn element_candidate(
        element: &MacControlElementSummary,
        target: &MacControlTargetQuery,
        snapshot: &MacControlSnapshot,
    ) -> MacControlElementCandidate {
        let window = element
            .window_id
            .as_deref()
            .and_then(|window_id| {
                snapshot
                    .windows
                    .iter()
                    .find(|window| window.id == window_id)
            })
            .cloned();
        MacControlElementCandidate {
            element: element.clone(),
            window: window.clone(),
            score: element_target_score(element, target),
            reasons: element_candidate_reasons(element, target, window.as_ref()),
        }
    }

    fn element_candidate_reasons(
        element: &MacControlElementSummary,
        target: &MacControlTargetQuery,
        window: Option<&MacControlWindowSummary>,
    ) -> Vec<String> {
        let mut reasons = Vec::new();
        if target
            .element_id
            .as_deref()
            .is_some_and(|query| !query.is_empty() && query == element.id)
        {
            reasons.push("elementId".to_string());
        }
        if let Some(query) = target.text.as_deref().filter(|query| !query.is_empty()) {
            if optional_eq_ci(element.label.as_deref(), query)
                || optional_eq_ci(element.value.as_deref(), query)
            {
                reasons.push("text:exact".to_string());
            } else {
                reasons.push("text:contains".to_string());
            }
        }
        if target
            .role
            .as_deref()
            .is_some_and(|query| !query.is_empty())
        {
            reasons.push("role".to_string());
        }
        if target
            .window_title
            .as_deref()
            .is_some_and(|query| !query.is_empty())
            && window.is_some()
        {
            reasons.push("windowTitle".to_string());
        }
        if element.focused {
            reasons.push("focused".to_string());
        }
        if element.enabled == Some(true) {
            reasons.push("enabled".to_string());
        }
        if element.actions.iter().any(|action| action == "AXPress") {
            reasons.push("pressable".to_string());
        }
        if element.bounds_points.is_some() {
            reasons.push("hasBounds".to_string());
        }
        if reasons.is_empty() {
            reasons.push("snapshot".to_string());
        }
        reasons
    }

    fn list_windows_for_request(
        request: &MacControlWindowsRequest,
    ) -> Result<Vec<MacControlWindowSummary>, String> {
        let windows = match request.window_scope {
            MacControlWindowsScope::Frontmost => frontmost_window_summaries()?,
            MacControlWindowsScope::All => all_window_summaries(&request.target)?,
        };
        if request
            .window_id
            .as_deref()
            .is_some_and(|value| !value.is_empty())
            || request
                .target
                .window_title
                .as_deref()
                .is_some_and(|value| !value.is_empty())
        {
            Ok(windows
                .into_iter()
                .filter(|window| window_matches_request(window, request))
                .collect())
        } else {
            Ok(windows)
        }
    }

    fn frontmost_window_summaries() -> Result<Vec<MacControlWindowSummary>, String> {
        let app = focused_app_element()?;
        let pid = ax_pid(app.as_ptr() as AXUIElementRef);
        let windows = copy_attribute(app.as_ptr() as AXUIElementRef, "AXWindows")
            .ok_or_else(|| "Focused app does not expose AXWindows.".to_string())?;
        Ok(cf_array_values(windows.as_ptr())
            .into_iter()
            .enumerate()
            .map(|(idx, window_ref)| {
                window_summary_for_app(
                    window_ref as AXUIElementRef,
                    &format!("win_{}", idx + 1),
                    pid,
                )
            })
            .collect())
    }

    fn all_window_summaries(
        target: &MacControlTargetQuery,
    ) -> Result<Vec<MacControlWindowSummary>, String> {
        let workspace = NSWorkspace::sharedWorkspace();
        let mut running = workspace.runningApplications().to_vec();
        if let Some(frontmost) = workspace.frontmostApplication() {
            if running
                .iter()
                .all(|app| app.processIdentifier() != frontmost.processIdentifier())
            {
                running.insert(0, frontmost);
            }
        }

        let mut seen = BTreeSet::new();
        let mut windows = Vec::new();
        for app in running {
            let summary = running_app_summary(&app);
            if !seen.insert(summary.pid) {
                continue;
            }
            if target_has_app_filter(target)
                && !running_app_summary_matches_target(&summary, target)
            {
                continue;
            }
            let Some(app_element) = app_element_for_pid(summary.pid) else {
                continue;
            };
            let Some(ax_windows) =
                copy_attribute(app_element.as_ptr() as AXUIElementRef, "AXWindows")
            else {
                continue;
            };
            for (idx, window_ref) in cf_array_values(ax_windows.as_ptr()).into_iter().enumerate() {
                let id = format!("win_{}_{}", summary.pid, idx + 1);
                windows.push(window_summary_for_app(
                    window_ref as AXUIElementRef,
                    &id,
                    Some(summary.pid),
                ));
            }
        }
        Ok(windows)
    }

    fn ensure_external_window_mutation(
        window: &MacControlWindowSummary,
        op: MacControlWindowsOp,
    ) -> Result<(), String> {
        if op == MacControlWindowsOp::List {
            return Ok(());
        }
        let current_pid = std::process::id();
        let Some(app_pid) = window.app_pid else {
            return Ok(());
        };
        if app_pid as u32 == current_pid {
            return Err(format!(
                "windows.{} cannot mutate Hope Agent's own window through Accessibility; AppKit window mutations must run on the main thread. Use an external app window, or add a dedicated main-thread self-window bridge.",
                windows_op_name(op)
            ));
        }
        Ok(())
    }

    fn windows_op_name(op: MacControlWindowsOp) -> &'static str {
        match op {
            MacControlWindowsOp::List => "list",
            MacControlWindowsOp::Focus => "focus",
            MacControlWindowsOp::Move => "move",
            MacControlWindowsOp::Resize => "resize",
            MacControlWindowsOp::Minimize => "minimize",
            MacControlWindowsOp::Close => "close",
        }
    }

    fn act_op_name(op: MacControlActOp) -> &'static str {
        match op {
            MacControlActOp::DryRun => "dry_run",
            MacControlActOp::PerformAction => "perform_action",
            MacControlActOp::Click => "click",
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

    fn handle_act(request: MacControlActRequest) -> Result<MacControlActResult, String> {
        let request = request.clamped();
        let mut target = None;
        let mut performed_action = None;
        let mut verification = None;
        let execution = match request.op {
            MacControlActOp::DryRun => {
                let intended_op = request.dry_run_op.unwrap_or(MacControlActOp::Click);
                target = resolve_dry_run_target(&request, intended_op)?;
                "DryRun".to_string()
            }
            MacControlActOp::PerformAction => {
                if target_query_is_empty(&request.target) {
                    return Err("act.perform_action requires a target.".to_string());
                }
                let requested_action = request
                    .ax_action
                    .as_deref()
                    .ok_or_else(|| "act.perform_action requires axAction.".to_string())?;
                let ax_action = normalize_perform_ax_action(requested_action).ok_or_else(|| {
                    format!("Unsupported act.perform_action axAction '{requested_action}'.")
                })?;
                let (element, summary, _) = resolve_element(
                    &request.target,
                    request.max_elements,
                    request.max_depth,
                    "act.perform_action",
                )?;
                perform_ax_action(element.as_ptr() as AXUIElementRef, &ax_action)?;
                target = Some(summary);
                performed_action = Some(ax_action.clone());
                ax_action
            }
            MacControlActOp::Click => {
                if target_query_is_empty(&request.target) {
                    return Err(
                        "act.click requires a target; use act.click_point for raw x/y coordinates."
                            .to_string(),
                    );
                }
                let (element, summary, _) = resolve_element(
                    &request.target,
                    request.max_elements,
                    request.max_depth,
                    "act.click",
                )?;
                let element_ref = element.as_ptr() as AXUIElementRef;
                target = Some(summary.clone());
                press_ax_or_click_center(element_ref, &summary, "act.click target")?
            }
            MacControlActOp::ClickPoint => {
                let (Some(x), Some(y)) = (request.x, request.y) else {
                    return Err("act.click_point requires x and y.".to_string());
                };
                if !target_query_is_empty(&request.target) {
                    return Err("act.click_point does not accept target; use act.click for AX element targets.".to_string());
                }
                post_mouse_click(screen_point(x, y, "act.click_point")?, MouseButton::Left)?;
                "CGEventClick".to_string()
            }
            MacControlActOp::MoveCursor => {
                let point = if target_query_is_empty(&request.target) {
                    let (Some(x), Some(y)) = (request.x, request.y) else {
                        return Err("act.move_cursor requires x and y or a target.".to_string());
                    };
                    screen_point(x, y, "act.move_cursor")?
                } else {
                    if request.x.is_some() || request.y.is_some() {
                        return Err(
                            "act.move_cursor accepts either target or x/y, not both.".to_string()
                        );
                    }
                    let (_element, summary, _) = resolve_element(
                        &request.target,
                        request.max_elements,
                        request.max_depth,
                        "act.move_cursor",
                    )?;
                    let point = point_for_element(&summary, "act.move_cursor target")?;
                    target = Some(summary);
                    point
                };
                post_mouse_move(
                    point,
                    motion_profile(&request, DEFAULT_MOTION_STEPS, DEFAULT_MOTION_DURATION_MS),
                )?;
                verification = Some(verify_pointer_position(point, "act.move_cursor"));
                "CGEventMoveCursor".to_string()
            }
            MacControlActOp::DoubleClick => {
                if target_query_is_empty(&request.target) {
                    return Err("act.double_click requires a target.".to_string());
                }
                let (_element, summary, _) = resolve_element(
                    &request.target,
                    request.max_elements,
                    request.max_depth,
                    "act.double_click",
                )?;
                let point = point_for_element(&summary, "act.double_click target")?;
                post_double_click(point)?;
                target = Some(summary);
                "CGEventDoubleClick".to_string()
            }
            MacControlActOp::RightClick => {
                if target_query_is_empty(&request.target) {
                    return Err("act.right_click requires a target.".to_string());
                }
                let (_element, summary, _) = resolve_element(
                    &request.target,
                    request.max_elements,
                    request.max_depth,
                    "act.right_click",
                )?;
                let point = point_for_element(&summary, "act.right_click target")?;
                post_mouse_click(point, MouseButton::Right)?;
                target = Some(summary);
                "CGEventRightClick".to_string()
            }
            MacControlActOp::Type => {
                let text = request
                    .text
                    .as_deref()
                    .ok_or_else(|| "act.type requires text.".to_string())?;
                let (element, summary) = if target_query_is_empty(&request.target) {
                    focused_element().ok_or_else(|| {
                        "act.type requires a focused text element or explicit target.".to_string()
                    })?
                } else {
                    let (element, summary, _) = resolve_type_element(
                        &request.target,
                        request.max_elements,
                        request.max_depth,
                        "act.type",
                    )?;
                    (element, summary)
                };
                let element_ref = element.as_ptr() as AXUIElementRef;
                let append_typing =
                    request.typing_profile.is_some() || request.typing_delay_ms.is_some();
                let before_value = append_typing.then(|| attribute_string(element_ref, "AXValue"));
                let execution = if append_typing {
                    focus_text_element_for_paste(element_ref, &summary)?;
                    post_unicode_text(text, typing_motion_profile(&request))?;
                    "CGEventUnicodeTyping".to_string()
                } else {
                    match set_ax_string(element_ref, "AXValue", text) {
                        Ok(()) => "AXSetValue".to_string(),
                        Err(error) => replace_text_via_clipboard(
                            element_ref,
                            &summary,
                            text,
                            "act.type fallback",
                            Some(error),
                        )?,
                    }
                };
                verification = Some(verify_ax_value(
                    element_ref,
                    text,
                    "act.type",
                    if append_typing {
                        AxValueVerificationMode::Append {
                            before: before_value.flatten(),
                        }
                    } else {
                        AxValueVerificationMode::Exact
                    },
                ));
                target = Some(summary);
                execution
            }
            MacControlActOp::Paste => {
                let text = request
                    .text
                    .as_deref()
                    .ok_or_else(|| "act.paste requires text.".to_string())?;
                let mut verify_element = None;
                let mut before_value = None;
                if target_query_is_empty(&request.target) {
                    if let Some((element, summary)) = focused_element() {
                        before_value =
                            attribute_string(element.as_ptr() as AXUIElementRef, "AXValue");
                        verify_element = Some(element);
                        target = Some(summary);
                    }
                } else {
                    let (element, summary, _) = resolve_type_element(
                        &request.target,
                        request.max_elements,
                        request.max_depth,
                        "act.paste",
                    )?;
                    let element_ref = element.as_ptr() as AXUIElementRef;
                    before_value = attribute_string(element_ref, "AXValue");
                    focus_text_element_for_paste(element_ref, &summary)?;
                    verify_element = Some(element);
                    target = Some(summary);
                }
                let execution = paste_text_via_clipboard(text)?;
                if let Some(element) = verify_element {
                    verification = Some(verify_ax_value(
                        element.as_ptr() as AXUIElementRef,
                        text,
                        "act.paste",
                        AxValueVerificationMode::Append {
                            before: before_value,
                        },
                    ));
                }
                execution
            }
            MacControlActOp::SetValue => {
                let value = request
                    .value
                    .as_deref()
                    .ok_or_else(|| "act.set_value requires value.".to_string())?;
                let (element, summary, _) = resolve_element(
                    &request.target,
                    request.max_elements,
                    request.max_depth,
                    "act.set_value",
                )?;
                let element_ref = element.as_ptr() as AXUIElementRef;
                let execution = match set_ax_string(element_ref, "AXValue", value) {
                    Ok(()) => "AXSetValue".to_string(),
                    Err(error) => {
                        if !is_text_input_element(&summary) {
                            return Err(format!(
                                "act.set_value AXSetValue failed for non-text target; pasteboard replace fallback is only allowed for text input elements: {error}"
                            ));
                        }
                        replace_text_via_clipboard(
                            element_ref,
                            &summary,
                            value,
                            "act.set_value fallback",
                            Some(error),
                        )?
                    }
                };
                verification = Some(verify_ax_value(
                    element_ref,
                    value,
                    "act.set_value",
                    AxValueVerificationMode::Exact,
                ));
                target = Some(summary);
                execution
            }
            MacControlActOp::Hotkey => {
                let keys = if request.keys.is_empty() {
                    vec![request.key.clone().unwrap_or_default()]
                } else {
                    request.keys.clone()
                };
                post_hotkey(&keys)?;
                "CGEventHotkey".to_string()
            }
            MacControlActOp::Press => {
                let keys = if request.keys.is_empty() {
                    vec![request.key.clone().unwrap_or_default()]
                } else {
                    request.keys.clone()
                };
                post_press_sequence(
                    &keys,
                    &request.modifiers,
                    request.repeat.unwrap_or(1),
                    request.hold_ms.unwrap_or(20),
                    request.interval_ms.unwrap_or(0),
                )?;
                "CGEventPress".to_string()
            }
            MacControlActOp::Scroll => {
                post_scroll(
                    request.delta_x.unwrap_or(0.0),
                    request.delta_y.unwrap_or(0.0),
                )?;
                "CGEventScroll".to_string()
            }
            MacControlActOp::Drag => {
                let (from, source_summary) = resolve_drag_source(&request)?;
                let (to, destination_summary) = resolve_drag_destination(&request)?;
                let modifiers = parse_modifier_keys(&request.modifiers, "act.drag modifiers")?;
                post_mouse_drag(
                    from,
                    to,
                    motion_profile(&request, DEFAULT_DRAG_STEPS, DEFAULT_DRAG_DURATION_MS),
                    &modifiers,
                )?;
                verification = Some(verify_pointer_position(to, "act.drag"));
                target = source_summary.or(destination_summary);
                "CGEventDrag".to_string()
            }
            MacControlActOp::Swipe => {
                let (from, source_summary) = resolve_swipe_source(&request)?;
                let (to, destination_summary) = resolve_swipe_destination(&request, from)?;
                let modifiers = parse_modifier_keys(&request.modifiers, "act.swipe modifiers")?;
                post_mouse_drag(
                    from,
                    to,
                    motion_profile(&request, DEFAULT_MOTION_STEPS, DEFAULT_MOTION_DURATION_MS),
                    &modifiers,
                )?;
                verification = Some(verify_pointer_position(to, "act.swipe"));
                target = source_summary.or(destination_summary);
                "CGEventSwipe".to_string()
            }
        };
        let preview = if request.op == MacControlActOp::DryRun || request.explain {
            Some(mac_control_act_preview(&request, target.as_ref()))
        } else {
            None
        };
        let snapshot = if request.op == MacControlActOp::DryRun || !request.include_snapshot {
            None
        } else {
            capture_ax_snapshot(MacControlSnapshotRequest {
                include_screenshot: false,
                max_elements: request.max_elements,
                max_depth: request.max_depth,
                ..Default::default()
            })
            .ok()
        };
        Ok(MacControlActResult {
            op: request.op,
            execution,
            performed_action,
            target,
            snapshot,
            verification,
            preview,
        })
    }

    fn resolve_dry_run_target(
        request: &MacControlActRequest,
        intended_op: MacControlActOp,
    ) -> Result<Option<MacControlElementSummary>, String> {
        match intended_op {
            MacControlActOp::DryRun => Ok(None),
            MacControlActOp::Click | MacControlActOp::PerformAction | MacControlActOp::SetValue => {
                if target_query_is_empty(&request.target) {
                    return Err(format!(
                        "act.dry_run dryRunOp={} requires a target.",
                        act_op_name(intended_op)
                    ));
                }
                let (_element, summary, _) = resolve_element(
                    &request.target,
                    request.max_elements,
                    request.max_depth,
                    "act.dry_run",
                )?;
                Ok(Some(summary))
            }
            MacControlActOp::DoubleClick | MacControlActOp::RightClick => {
                if target_query_is_empty(&request.target) {
                    return Err(format!(
                        "act.dry_run dryRunOp={} requires a target.",
                        act_op_name(intended_op)
                    ));
                }
                let (_element, summary, _) = resolve_element(
                    &request.target,
                    request.max_elements,
                    request.max_depth,
                    "act.dry_run",
                )?;
                point_for_element(&summary, "act.dry_run target")?;
                Ok(Some(summary))
            }
            MacControlActOp::Type => {
                if target_query_is_empty(&request.target) {
                    let (_element, summary) = focused_element().ok_or_else(|| {
                        "act.dry_run dryRunOp=type requires a focused text element or explicit target."
                            .to_string()
                    })?;
                    Ok(Some(summary))
                } else {
                    let (_element, summary, _) = resolve_type_element(
                        &request.target,
                        request.max_elements,
                        request.max_depth,
                        "act.dry_run",
                    )?;
                    Ok(Some(summary))
                }
            }
            MacControlActOp::Paste => {
                if target_query_is_empty(&request.target) {
                    Ok(focused_element().map(|(_element, summary)| summary))
                } else {
                    let (_element, summary, _) = resolve_type_element(
                        &request.target,
                        request.max_elements,
                        request.max_depth,
                        "act.dry_run",
                    )?;
                    Ok(Some(summary))
                }
            }
            MacControlActOp::ClickPoint => {
                let (Some(x), Some(y)) = (request.x, request.y) else {
                    return Err("act.dry_run dryRunOp=click_point requires x and y.".to_string());
                };
                screen_point(x, y, "act.dry_run click_point")?;
                Ok(None)
            }
            MacControlActOp::MoveCursor => {
                if target_query_is_empty(&request.target) {
                    let (Some(x), Some(y)) = (request.x, request.y) else {
                        return Err("act.dry_run dryRunOp=move_cursor requires x/y or a target."
                            .to_string());
                    };
                    screen_point(x, y, "act.dry_run move_cursor")?;
                    Ok(None)
                } else {
                    let (_element, summary, _) = resolve_element(
                        &request.target,
                        request.max_elements,
                        request.max_depth,
                        "act.dry_run",
                    )?;
                    point_for_element(&summary, "act.dry_run move_cursor target")?;
                    Ok(Some(summary))
                }
            }
            MacControlActOp::Hotkey => {
                let keys = if request.keys.is_empty() {
                    vec![request.key.clone().unwrap_or_default()]
                } else {
                    request.keys.clone()
                };
                validate_hotkey_keys(&keys)?;
                Ok(None)
            }
            MacControlActOp::Press => {
                let keys = if request.keys.is_empty() {
                    vec![request.key.clone().unwrap_or_default()]
                } else {
                    request.keys.clone()
                };
                validate_press_keys(&keys, &request.modifiers)?;
                Ok(None)
            }
            MacControlActOp::Scroll => Ok(None),
            MacControlActOp::Drag => {
                let (_from, source_summary) = resolve_drag_source(request)?;
                let (_to, destination_summary) = resolve_drag_destination(request)?;
                parse_modifier_keys(&request.modifiers, "act.drag modifiers")?;
                Ok(source_summary.or(destination_summary))
            }
            MacControlActOp::Swipe => {
                let (from, source_summary) = resolve_swipe_source(request)?;
                let (_to, destination_summary) = resolve_swipe_destination(request, from)?;
                parse_modifier_keys(&request.modifiers, "act.swipe modifiers")?;
                Ok(source_summary.or(destination_summary))
            }
        }
    }

    fn handle_menu(request: MacControlMenuRequest) -> Result<MacControlMenuResult, String> {
        let request = request.clamped();
        if request.op == MacControlMenuOp::Popover {
            return handle_menu_popover(request);
        }

        let menu_bar = menu_root_for_scope(request.scope)?;
        let menu_bar_ref = menu_bar.as_ptr() as AXUIElementRef;
        let items = menu_items_for_scope(menu_bar_ref, request.scope, request.max_depth);
        let mut warnings = Vec::new();
        let mut popovers = Vec::new();
        let mut screenshot = None;
        let clicked = if request.op == MacControlMenuOp::Click {
            let clicked = if !request.path.is_empty() {
                click_menu_path(menu_bar_ref, &request.path)?
            } else if let Some(index) = request.menu_index {
                click_menu_index(menu_bar_ref, request.scope, index)?
            } else {
                return Err("menu.click requires path or menuIndex.".to_string());
            };
            if request.verify {
                if request.scope == MacControlMenuScope::System {
                    let mut verify_request = request.clone();
                    verify_request.op = MacControlMenuOp::Popover;
                    if verify_request.app_hint.is_none() {
                        verify_request.app_hint = menu_item_primary_text(&clicked);
                    }
                    match handle_menu_popover(verify_request) {
                        Ok(result) => {
                            warnings.extend(result.warnings);
                            popovers = result.popovers;
                            screenshot = result.screenshot;
                            if popovers.is_empty() {
                                warnings.push(
                                    "menu.click verify did not find a likely status-item popover."
                                        .to_string(),
                                );
                            }
                        }
                        Err(error) => {
                            warnings.push(format!("menu.click verify failed: {error}"));
                        }
                    }
                } else {
                    warnings.push(
                        "menu.click verify is only supported for scope=\"system\" status items."
                            .to_string(),
                    );
                }
            }
            Some(clicked)
        } else {
            None
        };

        Ok(MacControlMenuResult {
            op: request.op,
            scope: request.scope,
            path: request.path,
            items,
            clicked,
            popovers,
            screenshot,
            warnings,
        })
    }

    fn handle_menu_popover(request: MacControlMenuRequest) -> Result<MacControlMenuResult, String> {
        let mut warnings = Vec::new();
        let displays = match display_summaries() {
            Ok(displays) => displays,
            Err(error) => {
                warnings.push(error);
                Vec::new()
            }
        };
        let (screenshot, ocr_blocks) = capture_menu_popover_ocr_blocks(&request, &mut warnings);

        let workspace = NSWorkspace::sharedWorkspace();
        let mut running = workspace.runningApplications().to_vec();
        if let Some(frontmost) = workspace.frontmostApplication() {
            if running
                .iter()
                .all(|app| app.processIdentifier() != frontmost.processIdentifier())
            {
                running.insert(0, frontmost);
            }
        }

        let mut seen = BTreeSet::new();
        let mut popovers = Vec::new();
        for app in running {
            let running_summary = running_app_summary(&app);
            if !seen.insert(running_summary.pid) {
                continue;
            }
            let Some(app_element) = app_element_for_pid(running_summary.pid) else {
                continue;
            };
            let Some(ax_windows) =
                copy_attribute(app_element.as_ptr() as AXUIElementRef, "AXWindows")
            else {
                continue;
            };
            let app_summary = MacControlAppSummary {
                pid: running_summary.pid,
                bundle_id: running_summary.bundle_id.clone(),
                name: running_summary.name.clone(),
            };
            for (idx, window_ref) in cf_array_values(ax_windows.as_ptr()).into_iter().enumerate() {
                let id = format!("win_{}_{}", running_summary.pid, idx + 1);
                let summary = window_summary_for_app(
                    window_ref as AXUIElementRef,
                    &id,
                    Some(running_summary.pid),
                );
                if let Some(candidate) = score_menu_popover_window(
                    summary,
                    Some(app_summary.clone()),
                    &displays,
                    &ocr_blocks,
                    request.app_hint.as_deref(),
                ) {
                    popovers.push(candidate);
                }
            }
        }

        popovers.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| right.window.focused.cmp(&left.window.focused))
                .then_with(|| left.window.id.cmp(&right.window.id))
        });
        if popovers.len() > request.limit {
            warnings.push(format!(
                "menu.popover matched {} candidates; returning top {}.",
                popovers.len(),
                request.limit
            ));
            popovers.truncate(request.limit);
        }
        if popovers.is_empty() {
            warnings.push(
                "No likely menu bar popover windows were found. Open a status item popover and retry, or pass appHint for a specific menu bar app.".to_string(),
            );
        }

        Ok(MacControlMenuResult {
            op: request.op,
            scope: request.scope,
            path: request.path,
            items: Vec::new(),
            clicked: None,
            popovers,
            screenshot,
            warnings,
        })
    }

    fn capture_menu_popover_ocr_blocks(
        request: &MacControlMenuRequest,
        warnings: &mut Vec<String>,
    ) -> (
        Option<MacControlScreenshotSummary>,
        Vec<MenuPopoverOcrBlock>,
    ) {
        if !request.include_ocr {
            return (None, Vec::new());
        }

        let captured = match capture_display_frame_bytes(None) {
            Ok(captured) => captured,
            Err(error) => {
                warnings.push(format!("menu.popover OCR screenshot failed: {error}"));
                return (None, Vec::new());
            }
        };
        let snapshot_id = ha_core::mac_control::new_snapshot_id();
        let mut screenshot = match ha_core::mac_control::store_screenshot_jpeg(
            &snapshot_id,
            &captured.jpeg,
            captured.width_px,
            captured.height_px,
        ) {
            Ok(screenshot) => screenshot,
            Err(error) => {
                warnings.push(format!("menu.popover OCR screenshot write failed: {error}"));
                return (None, Vec::new());
            }
        };
        apply_capture_metadata_to_screenshot(&mut screenshot, &captured);

        let raw_blocks = match handle_ocr(MacControlOcrRequest {
            screenshot: screenshot.clone(),
            languages: request.languages.clone(),
            recognition_level: request.recognition_level,
        }) {
            Ok(blocks) => blocks,
            Err(error) => {
                warnings.push(format!("menu.popover OCR failed: {error}"));
                return (Some(screenshot), Vec::new());
            }
        };

        let min_confidence = request.min_confidence.unwrap_or(0.0);
        let blocks = raw_blocks
            .into_iter()
            .filter(|block| block.confidence.is_finite() && block.confidence >= min_confidence)
            .filter_map(|block| menu_popover_ocr_block_to_screen(block, &captured))
            .collect();
        (Some(screenshot), blocks)
    }

    fn menu_popover_ocr_block_to_screen(
        block: MacControlOcrRawTextBlock,
        captured: &CapturedDesktopFrame,
    ) -> Option<MenuPopoverOcrBlock> {
        let frame = captured.bounds_points?;
        let scale = captured.scale.filter(|value| *value > 0.0).unwrap_or(1.0);
        Some(MenuPopoverOcrBlock {
            text: block.text,
            screen_bounds: MacControlBounds {
                x: frame.x + block.image_bounds.x / scale,
                y: frame.y + block.image_bounds.y / scale,
                width: block.image_bounds.width / scale,
                height: block.image_bounds.height / scale,
            },
        })
    }

    fn score_menu_popover_window(
        window: MacControlWindowSummary,
        app: Option<MacControlAppSummary>,
        displays: &[MacControlDisplaySummary],
        ocr_blocks: &[MenuPopoverOcrBlock],
        app_hint: Option<&str>,
    ) -> Option<MacControlMenuPopoverCandidate> {
        let bounds = window.bounds_points?;
        if bounds.width < 40.0 || bounds.height < 20.0 {
            return None;
        }

        let ocr_text = menu_popover_ocr_text_for_bounds(ocr_blocks, bounds);
        let mut score: i32 = 0;
        let mut reasons = Vec::new();
        let role = window
            .role
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let subrole = window
            .subrole
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let title = window.title.as_deref().unwrap_or_default();
        let app_name = app
            .as_ref()
            .and_then(|summary| summary.name.as_deref())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let bundle_id = app
            .as_ref()
            .and_then(|summary| summary.bundle_id.as_deref())
            .unwrap_or_default()
            .to_ascii_lowercase();

        if role.contains("window") {
            score += 5;
        }
        if role.contains("popover") || subrole.contains("popover") {
            score += 35;
            reasons.push("popoverRole".to_string());
        }
        if subrole.contains("floating")
            || subrole.contains("systemdialog")
            || subrole.contains("dialog")
            || subrole.contains("unknown")
        {
            score += 18;
            reasons.push("panelSubrole".to_string());
        }
        if window.focused {
            score += 12;
            reasons.push("focused".to_string());
        }
        if title.trim().is_empty() {
            score += 8;
            reasons.push("untitledPanel".to_string());
        }

        if let Some(display) = menu_popover_display_for_bounds(bounds, displays) {
            let display_frame = display.frame_points;
            let display_area = display_frame.width.max(1.0) * display_frame.height.max(1.0);
            let area_ratio = (bounds.width * bounds.height) / display_area;
            if bounds.y >= display_frame.y - 8.0 && bounds.y <= display_frame.y + 220.0 {
                score += 25;
                reasons.push("nearMenuBar".to_string());
            }
            if bounds.x + bounds.width / 2.0 >= display_frame.x + display_frame.width * 0.45 {
                score += 6;
                reasons.push("rightMenuBarArea".to_string());
            }
            if area_ratio <= 0.35
                && (60.0..=1100.0).contains(&bounds.width)
                && (30.0..=900.0).contains(&bounds.height)
            {
                score += 14;
                reasons.push("popoverSized".to_string());
            } else if area_ratio > 0.55 {
                score -= 25;
            }
        } else if bounds.y <= 220.0 {
            score += 18;
            reasons.push("nearMenuBar".to_string());
        }

        if menu_popover_host_app_likely(&app_name, &bundle_id) {
            score += 22;
            reasons.push("menuBarHostApp".to_string());
        }
        if !ocr_text.is_empty() {
            score += 10;
            reasons.push("ocrText".to_string());
        }
        if app_hint
            .is_some_and(|hint| menu_popover_hint_matches(hint, &window, app.as_ref(), &ocr_text))
        {
            score += 30;
            reasons.push("appHint".to_string());
        }

        let normal_app_window = !title.trim().is_empty()
            && !reasons.iter().any(|reason| reason == "nearMenuBar")
            && !reasons.iter().any(|reason| reason == "popoverRole");
        if normal_app_window && score < 55 {
            return None;
        }
        if score < 30 {
            return None;
        }

        Some(MacControlMenuPopoverCandidate {
            window,
            app,
            score: score.clamp(0, 100) as u8,
            reasons,
            ocr_text,
        })
    }

    fn menu_popover_ocr_text_for_bounds(
        ocr_blocks: &[MenuPopoverOcrBlock],
        bounds: MacControlBounds,
    ) -> Vec<String> {
        let mut text = Vec::new();
        for block in ocr_blocks {
            let block_bounds = block.screen_bounds;
            let center_x = block_bounds.x + block_bounds.width / 2.0;
            let center_y = block_bounds.y + block_bounds.height / 2.0;
            if point_in_bounds(center_x, center_y, bounds) || bounds_intersect(block_bounds, bounds)
            {
                let trimmed = block.text.trim();
                if !trimmed.is_empty() && !text.iter().any(|existing| existing == trimmed) {
                    text.push(trimmed.to_string());
                }
            }
        }
        text
    }

    fn menu_popover_display_for_bounds<'a>(
        bounds: MacControlBounds,
        displays: &'a [MacControlDisplaySummary],
    ) -> Option<&'a MacControlDisplaySummary> {
        let center_x = bounds.x + bounds.width / 2.0;
        let center_y = bounds.y + bounds.height / 2.0;
        displays
            .iter()
            .find(|display| point_in_bounds(center_x, center_y, display.frame_points))
            .or_else(|| {
                displays
                    .iter()
                    .find(|display| bounds_intersect(bounds, display.frame_points))
            })
    }

    fn menu_popover_host_app_likely(app_name: &str, bundle_id: &str) -> bool {
        const HINTS: &[&str] = &[
            "systemuiserver",
            "controlcenter",
            "control center",
            "notificationcenter",
            "notification center",
            "bartender",
            "istat",
            "menubar",
            "menu bar",
            "wifi",
            "bluetooth",
            "battery",
            "clock",
        ];
        HINTS
            .iter()
            .any(|hint| app_name.contains(hint) || bundle_id.contains(hint))
    }

    fn menu_popover_hint_matches(
        hint: &str,
        window: &MacControlWindowSummary,
        app: Option<&MacControlAppSummary>,
        ocr_text: &[String],
    ) -> bool {
        let hint = hint.to_ascii_lowercase();
        if hint.trim().is_empty() {
            return false;
        }
        let string_matches = |value: Option<&str>| {
            value
                .map(|value| value.to_ascii_lowercase().contains(&hint))
                .unwrap_or(false)
        };
        string_matches(window.title.as_deref())
            || app.is_some_and(|app| {
                string_matches(app.name.as_deref()) || string_matches(app.bundle_id.as_deref())
            })
            || ocr_text
                .iter()
                .any(|text| text.to_ascii_lowercase().contains(&hint))
    }

    fn menu_root_for_scope(scope: MacControlMenuScope) -> Result<CfOwned, String> {
        match scope {
            MacControlMenuScope::App => {
                let app = focused_app_element()?;
                copy_attribute(app.as_ptr() as AXUIElementRef, "AXMenuBar")
                    .ok_or_else(|| "Focused app does not expose an AXMenuBar.".to_string())
            }
            MacControlMenuScope::System => {
                let system = unsafe { AXUIElementCreateSystemWide() };
                let system = CfOwned::new(system as CFTypeRef).ok_or_else(|| {
                    "Unable to create the system Accessibility element.".to_string()
                })?;
                copy_attribute(system.as_ptr() as AXUIElementRef, "AXExtrasMenuBar").ok_or_else(
                    || "System menu bar extras are unavailable through Accessibility.".to_string(),
                )
            }
        }
    }

    fn handle_clipboard(
        request: MacControlClipboardRequest,
    ) -> Result<MacControlClipboardResult, String> {
        let request = request.clamped();
        let mut clipboard =
            arboard::Clipboard::new().map_err(|e| format!("Failed to access clipboard: {e}"))?;
        match request.op {
            MacControlClipboardOp::Get => {
                let text = clipboard
                    .get_text()
                    .map_err(|e| format!("Clipboard does not contain UTF-8 text: {e}"))?;
                let (text, text_len, truncated) = truncate_clipboard_text(text, request.max_chars);
                Ok(MacControlClipboardResult {
                    op: request.op,
                    text: Some(text),
                    text_len,
                    truncated,
                    changed: false,
                })
            }
            MacControlClipboardOp::Set => {
                let text = request
                    .text
                    .ok_or_else(|| "clipboard.set requires text.".to_string())?;
                let text_len = request
                    .text_original_len
                    .unwrap_or_else(|| text.chars().count());
                let truncated = request.text_truncated;
                clipboard
                    .set_text(text)
                    .map_err(|e| format!("Failed to set clipboard text: {e}"))?;
                Ok(MacControlClipboardResult {
                    op: request.op,
                    text: None,
                    text_len,
                    truncated,
                    changed: true,
                })
            }
            MacControlClipboardOp::Clear => {
                clipboard
                    .clear()
                    .map_err(|e| format!("Failed to clear clipboard: {e}"))?;
                Ok(MacControlClipboardResult {
                    op: request.op,
                    text: None,
                    text_len: 0,
                    truncated: false,
                    changed: true,
                })
            }
        }
    }

    fn truncate_clipboard_text(text: String, max_chars: usize) -> (String, usize, bool) {
        let text_len = text.chars().count();
        if text_len <= max_chars {
            return (text, text_len, false);
        }
        (text.chars().take(max_chars).collect(), text_len, true)
    }

    fn focus_text_element_for_paste(
        element: AXUIElementRef,
        summary: &MacControlElementSummary,
    ) -> Result<(), String> {
        if set_ax_bool(element, "AXFocused", true).is_ok() {
            thread::sleep(Duration::from_millis(40));
            return Ok(());
        }
        let point = point_for_element(summary, "act.paste target")?;
        post_mouse_click(point, MouseButton::Left)?;
        thread::sleep(Duration::from_millis(80));
        Ok(())
    }

    fn paste_text_via_clipboard(text: &str) -> Result<String, String> {
        let pasteboard = NSPasteboard::generalPasteboard();
        let previous_items = copy_pasteboard_items(&pasteboard)?;
        if let Err(error) = stage_text_on_pasteboard(&pasteboard, text) {
            let restore_status = restore_pasteboard_items(&pasteboard, &previous_items);
            return Err(format!(
                "Failed to stage paste text on clipboard ({restore_status}): {error}"
            ));
        }

        let paste_result = post_hotkey(&["cmd".to_string(), "v".to_string()]);
        thread::sleep(Duration::from_millis(120));
        let restore_status = restore_pasteboard_items(&pasteboard, &previous_items);

        match paste_result {
            Ok(()) => Ok(format!(
                "PasteboardCommandV(clipboard_restore={restore_status})"
            )),
            Err(error) => Err(format!(
                "Paste hotkey failed after clipboard staging ({restore_status}): {error}"
            )),
        }
    }

    fn replace_text_via_clipboard(
        element: AXUIElementRef,
        summary: &MacControlElementSummary,
        text: &str,
        label: &str,
        ax_error: Option<String>,
    ) -> Result<String, String> {
        focus_text_element_for_paste(element, summary)
            .map_err(|error| format!("{label} could not focus target: {error}"))?;
        post_hotkey(&["cmd".to_string(), "a".to_string()])
            .map_err(|error| format!("{label} select-all failed: {error}"))?;
        thread::sleep(Duration::from_millis(80));
        let paste_execution = paste_text_via_clipboard(text)
            .map_err(|error| format!("{label} paste fallback failed: {error}"))?;
        Ok(match ax_error {
            Some(error) => format!(
                "AXSetValueFailed+PasteboardReplace({}; {paste_execution})",
                truncate_for_error(&error, 96)
            ),
            None => format!("PasteboardReplace({paste_execution})"),
        })
    }

    fn copy_pasteboard_items(
        pasteboard: &NSPasteboard,
    ) -> Result<Vec<Retained<NSPasteboardItem>>, String> {
        let Some(items) = pasteboard.pasteboardItems() else {
            return Ok(Vec::new());
        };
        let mut copies = Vec::new();
        for item in items.to_vec() {
            let copy = NSPasteboardItem::new();
            let types = item.types().to_vec();
            if types.is_empty() {
                return Err(
                    "act.paste cannot safely preserve a pasteboard item with no declared types."
                        .to_string(),
                );
            }
            for pasteboard_type in types {
                let data = item.dataForType(&pasteboard_type).ok_or_else(|| {
                    "act.paste cannot safely preserve the current pasteboard item data.".to_string()
                })?;
                if !copy.setData_forType(&data, &pasteboard_type) {
                    return Err(
                        "act.paste failed to copy current pasteboard item data.".to_string()
                    );
                }
            }
            copies.push(copy);
        }
        Ok(copies)
    }

    fn stage_text_on_pasteboard(pasteboard: &NSPasteboard, text: &str) -> Result<(), String> {
        let item = NSPasteboardItem::new();
        let text = NSString::from_str(text);
        let string_type = NSString::from_str("public.utf8-plain-text");
        if !item.setString_forType(&text, &string_type) {
            return Err("NSPasteboardItem refused the staged UTF-8 text.".to_string());
        }
        pasteboard.clearContents();
        let items = vec![item];
        if write_pasteboard_items(pasteboard, &items) {
            Ok(())
        } else {
            Err("NSPasteboard refused the staged UTF-8 text item.".to_string())
        }
    }

    fn restore_pasteboard_items(
        pasteboard: &NSPasteboard,
        items: &[Retained<NSPasteboardItem>],
    ) -> &'static str {
        pasteboard.clearContents();
        if items.is_empty() {
            return "restored_empty";
        }
        if write_pasteboard_items(pasteboard, items) {
            "restored_items"
        } else {
            "restore_failed"
        }
    }

    fn write_pasteboard_items(
        pasteboard: &NSPasteboard,
        items: &[Retained<NSPasteboardItem>],
    ) -> bool {
        let writing_items = items
            .iter()
            .map(|item| ProtocolObject::<dyn NSPasteboardWriting>::from_ref(&**item))
            .collect::<Vec<_>>();
        let objects = NSArray::from_slice(&writing_items);
        pasteboard.writeObjects(&objects)
    }

    fn handle_dialog(request: MacControlDialogRequest) -> Result<MacControlDialogResult, String> {
        let request = request.clamped();
        let snapshot = capture_ax_snapshot(MacControlSnapshotRequest {
            include_screenshot: false,
            max_elements: request.max_elements,
            max_depth: request.max_depth,
            ..Default::default()
        })?;
        if !frontmost_app_matches_act_target(&snapshot, &request.target) {
            return Err("Frontmost app did not match the dialog target.".to_string());
        }

        let dialogs = dialog_summaries(&snapshot, &request.target);
        let mut acted_button = None;
        let mut acted_field = None;
        let mut file_dialog = None;
        let mut execution = None;
        let mut warnings = Vec::new();
        match request.op {
            MacControlDialogOp::Inspect | MacControlDialogOp::List => {}
            MacControlDialogOp::Accept
            | MacControlDialogOp::Dismiss
            | MacControlDialogOp::Click => {
                if let Some(button) = select_dialog_button(&dialogs, &request) {
                    let element = resolve_element_by_summary(
                        &button,
                        request.max_elements,
                        request.max_depth,
                    )?;
                    press_dialog_button(element.as_ptr() as AXUIElementRef, &button)?;
                    acted_button = Some(button);
                    execution = Some("AXPressOrCGEvent".to_string());
                } else if request.op == MacControlDialogOp::Dismiss && request.force {
                    post_hotkey(&["escape".to_string()])?;
                    execution = Some("CGEventEscape".to_string());
                } else {
                    return Err(format!(
                        "No dialog button matched dialog.{}.",
                        dialog_op_name(request.op)
                    ));
                }
            }
            MacControlDialogOp::Input => {
                let text = request
                    .text
                    .as_deref()
                    .ok_or_else(|| "dialog.input requires text.".to_string())?;
                let field = select_dialog_field(&dialogs, &request)
                    .ok_or_else(|| "No dialog text field matched dialog.input.".to_string())?;
                let element =
                    resolve_element_by_summary(&field, request.max_elements, request.max_depth)?;
                let element_ref = element.as_ptr() as AXUIElementRef;
                let action = if request.clear {
                    match set_ax_string(element_ref, "AXValue", text) {
                        Ok(()) => "AXSetValue".to_string(),
                        Err(error) => replace_text_via_clipboard(
                            element_ref,
                            &field,
                            text,
                            "dialog.input fallback",
                            Some(error),
                        )?,
                    }
                } else {
                    focus_text_element_for_paste(element_ref, &field)?;
                    paste_text_via_clipboard(text)?
                };
                acted_field = Some(field);
                execution = Some(action);
            }
            MacControlDialogOp::File => {
                let mut result = handle_file_dialog(&request, &mut warnings)?;
                acted_field = result.name_field.clone();
                execution = Some(
                    result
                        .path_navigation
                        .clone()
                        .unwrap_or_else(|| "DialogFileNoNavigation".to_string()),
                );

                let post_snapshot = capture_ax_snapshot(MacControlSnapshotRequest {
                    include_screenshot: false,
                    max_elements: request.max_elements,
                    max_depth: request.max_depth,
                    ..Default::default()
                })?;
                let post_dialogs = dialog_summaries(&post_snapshot, &request.target);
                match select_dialog_file_button(&post_dialogs, &request)? {
                    DialogFileButtonSelection::Skip => {}
                    DialogFileButtonSelection::Press(button) => {
                        let should_verify_close =
                            dialog_file_button_should_close(&request, &button);
                        let element = resolve_element_by_summary(
                            &button,
                            request.max_elements,
                            request.max_depth,
                        )?;
                        press_dialog_button(element.as_ptr() as AXUIElementRef, &button)?;
                        result.selected_button = element_display_text(&button);
                        execution = Some(format!(
                            "{}+AXPressOrCGEvent",
                            execution.as_deref().unwrap_or("DialogFile")
                        ));
                        if should_verify_close {
                            verify_dialog_file_closed(&request, &mut warnings);
                        }
                        acted_button = Some(button);
                    }
                }
                file_dialog = Some(result);
            }
        }

        Ok(MacControlDialogResult {
            op: request.op,
            dialogs,
            acted_button,
            acted_field,
            file_dialog,
            snapshot: request.include_snapshot.then_some(snapshot),
            execution,
            warnings,
        })
    }

    fn dialog_summaries(
        snapshot: &MacControlSnapshot,
        target: &MacControlTargetQuery,
    ) -> Vec<MacControlDialogSummary> {
        let mut dialogs = snapshot
            .windows
            .iter()
            .filter(|window| dialog_window_matches(window, target, snapshot))
            .map(|window| dialog_summary_for_window(snapshot, window))
            .collect::<Vec<_>>();
        dialogs.extend(
            snapshot
                .elements
                .iter()
                .filter(|element| dialog_element_matches(element, target, snapshot))
                .map(|element| dialog_summary_for_element(snapshot, element)),
        );
        dialogs.sort_by_key(|dialog| {
            if is_dialog_window(&dialog.window) {
                0
            } else {
                1
            }
        });
        dialogs
    }

    fn dialog_window_matches(
        window: &MacControlWindowSummary,
        target: &MacControlTargetQuery,
        snapshot: &MacControlSnapshot,
    ) -> bool {
        if !window_matches_query(window, target, snapshot) {
            return false;
        }
        is_dialog_window(window)
    }

    fn is_dialog_window(window: &MacControlWindowSummary) -> bool {
        let role = window
            .role
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let subrole = window
            .subrole
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        role.contains("dialog")
            || role.contains("sheet")
            || role.contains("systemdialog")
            || role.contains("popover")
            || subrole.contains("dialog")
            || subrole.contains("sheet")
            || subrole.contains("systemdialog")
            || subrole.contains("popover")
    }

    fn window_matches_query(
        window: &MacControlWindowSummary,
        target: &MacControlTargetQuery,
        snapshot: &MacControlSnapshot,
    ) -> bool {
        if !window_title_matches(window.title.as_deref(), target) {
            return false;
        }
        if target
            .text
            .as_deref()
            .filter(|query| !query.is_empty())
            .is_some_and(|query| {
                !snapshot
                    .elements
                    .iter()
                    .filter(|element| element.window_id.as_deref() == Some(window.id.as_str()))
                    .any(|element| {
                        contains_ci(element.label.as_deref(), Some(query))
                            || contains_ci(element.value.as_deref(), Some(query))
                    })
            })
        {
            return false;
        }
        true
    }

    fn dialog_element_matches(
        element: &MacControlElementSummary,
        target: &MacControlTargetQuery,
        snapshot: &MacControlSnapshot,
    ) -> bool {
        if !is_dialog_element(element) {
            return false;
        }
        if !target
            .element_id
            .as_deref()
            .filter(|query| !query.is_empty())
            .map(|query| element.id == query)
            .unwrap_or(true)
        {
            return false;
        }
        if !contains_ci(element.role.as_deref(), target.role.as_deref()) {
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
                contains_ci(element.label.as_deref(), Some(query))
                    || contains_ci(element.value.as_deref(), Some(query))
                    || dialog_parent_window(element, snapshot).is_some_and(|window| {
                        string_matches(window.title.as_deref(), query, target.window_title_match)
                    })
            })
            .unwrap_or(true)
        {
            return false;
        }
        if !target
            .text
            .as_deref()
            .filter(|query| !query.is_empty())
            .map(|query| {
                contains_ci(element.label.as_deref(), Some(query))
                    || contains_ci(element.value.as_deref(), Some(query))
                    || dialog_elements_for_root(snapshot, element)
                        .iter()
                        .any(|candidate| {
                            contains_ci(candidate.label.as_deref(), Some(query))
                                || contains_ci(candidate.value.as_deref(), Some(query))
                        })
            })
            .unwrap_or(true)
        {
            return false;
        }
        true
    }

    fn is_dialog_element(element: &MacControlElementSummary) -> bool {
        let role = element
            .role
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        role.contains("dialog")
            || role.contains("sheet")
            || role.contains("systemdialog")
            || role.contains("popover")
    }

    fn dialog_parent_window<'a>(
        element: &MacControlElementSummary,
        snapshot: &'a MacControlSnapshot,
    ) -> Option<&'a MacControlWindowSummary> {
        let window_id = element.window_id.as_deref()?;
        snapshot
            .windows
            .iter()
            .find(|window| window.id == window_id)
    }

    fn dialog_summary_for_window(
        snapshot: &MacControlSnapshot,
        window: &MacControlWindowSummary,
    ) -> MacControlDialogSummary {
        let elements = snapshot
            .elements
            .iter()
            .filter(|element| element.window_id.as_deref() == Some(window.id.as_str()))
            .collect::<Vec<_>>();
        let buttons = elements
            .iter()
            .filter(|element| is_button_element(element))
            .map(|element| (*element).clone())
            .collect::<Vec<_>>();
        let fields = elements
            .iter()
            .filter(|element| is_text_input_element(element))
            .map(|element| (*element).clone())
            .collect::<Vec<_>>();
        let text = elements
            .iter()
            .filter(|element| is_dialog_text_element(element))
            .filter_map(|element| {
                element
                    .label
                    .clone()
                    .or_else(|| element.value.clone())
                    .filter(|value| !value.is_empty())
            })
            .collect::<Vec<_>>();
        MacControlDialogSummary {
            window: window.clone(),
            text,
            buttons,
            fields,
        }
    }

    fn dialog_summary_for_element(
        snapshot: &MacControlSnapshot,
        root: &MacControlElementSummary,
    ) -> MacControlDialogSummary {
        let elements = dialog_elements_for_root(snapshot, root);
        let buttons = elements
            .iter()
            .filter(|element| is_button_element(element))
            .map(|element| (*element).clone())
            .collect::<Vec<_>>();
        let fields = elements
            .iter()
            .filter(|element| is_text_input_element(element))
            .map(|element| (*element).clone())
            .collect::<Vec<_>>();
        let text = elements
            .iter()
            .filter(|element| is_dialog_text_element(element))
            .filter_map(|element| {
                element
                    .label
                    .clone()
                    .or_else(|| element.value.clone())
                    .filter(|value| !value.is_empty())
            })
            .collect::<Vec<_>>();
        MacControlDialogSummary {
            window: MacControlWindowSummary {
                id: root.id.clone(),
                app_pid: dialog_parent_window(root, snapshot)
                    .and_then(|window| window.app_pid)
                    .or_else(|| snapshot.frontmost_app.as_ref().map(|app| app.pid)),
                role: root.role.clone(),
                subrole: None,
                title: root.label.clone().or_else(|| root.value.clone()),
                focused: root.focused,
                bounds_points: root.bounds_points,
            },
            text,
            buttons,
            fields,
        }
    }

    fn dialog_elements_for_root<'a>(
        snapshot: &'a MacControlSnapshot,
        root: &'a MacControlElementSummary,
    ) -> Vec<&'a MacControlElementSummary> {
        let mut elements = vec![root];
        let root_index = snapshot
            .elements
            .iter()
            .position(|element| element.id == root.id);
        let root_window_id = root.window_id.as_deref();
        if let (Some(root_index), Some(root_window_id)) = (root_index, root_window_id) {
            for element in snapshot.elements.iter().skip(root_index + 1) {
                if element.window_id.as_deref() != Some(root_window_id) {
                    break;
                }
                if element.id != root.id && is_dialog_element(element) {
                    break;
                }
                if element_belongs_to_dialog_root(element, root) {
                    elements.push(element);
                }
            }
        }
        if elements.len() == 1 {
            elements.extend(snapshot.elements.iter().filter(|element| {
                element.id != root.id
                    && element.window_id.as_deref() == root_window_id
                    && element_belongs_to_dialog_root(element, root)
            }));
        }
        elements
    }

    fn element_belongs_to_dialog_root(
        element: &MacControlElementSummary,
        root: &MacControlElementSummary,
    ) -> bool {
        if element.id == root.id {
            return true;
        }
        let (Some(root_bounds), Some(bounds)) = (root.bounds_points, element.bounds_points) else {
            return false;
        };
        let center_x = bounds.x + bounds.width / 2.0;
        let center_y = bounds.y + bounds.height / 2.0;
        let tolerance = 2.0;
        center_x >= root_bounds.x - tolerance
            && center_x <= root_bounds.x + root_bounds.width + tolerance
            && center_y >= root_bounds.y - tolerance
            && center_y <= root_bounds.y + root_bounds.height + tolerance
    }

    fn is_button_element(element: &MacControlElementSummary) -> bool {
        element
            .role
            .as_deref()
            .map(|role| role.to_ascii_lowercase().contains("button"))
            .unwrap_or(false)
    }

    fn is_dialog_text_element(element: &MacControlElementSummary) -> bool {
        let Some(role) = element.role.as_deref().map(str::to_ascii_lowercase) else {
            return false;
        };
        role.contains("statictext") || role.contains("text")
    }

    fn select_dialog_button(
        dialogs: &[MacControlDialogSummary],
        request: &MacControlDialogRequest,
    ) -> Option<MacControlElementSummary> {
        let explicit = request
            .button_text
            .as_deref()
            .or(request.target.text.as_deref())
            .filter(|value| !value.is_empty());
        if let Some(query) = explicit {
            if query.eq_ignore_ascii_case("default") {
                return select_dialog_default_button(dialogs);
            }
            return dialogs
                .iter()
                .flat_map(|dialog| dialog.buttons.iter())
                .filter(|button| button.enabled != Some(false))
                .find(|button| element_label_matches(button, query))
                .cloned();
        }

        let patterns = match request.op {
            MacControlDialogOp::Accept => ACCEPT_DIALOG_BUTTONS,
            MacControlDialogOp::Dismiss => DISMISS_DIALOG_BUTTONS,
            MacControlDialogOp::Click
            | MacControlDialogOp::File
            | MacControlDialogOp::Input
            | MacControlDialogOp::Inspect
            | MacControlDialogOp::List => &[],
        };
        dialogs
            .iter()
            .flat_map(|dialog| dialog.buttons.iter())
            .filter(|button| button.enabled != Some(false))
            .max_by_key(|button| dialog_button_score(button, patterns))
            .filter(|button| dialog_button_has_label_match(button, patterns))
            .cloned()
    }

    fn select_dialog_default_button(
        dialogs: &[MacControlDialogSummary],
    ) -> Option<MacControlElementSummary> {
        dialogs
            .iter()
            .flat_map(|dialog| dialog.buttons.iter())
            .filter(|button| button.enabled != Some(false))
            .max_by_key(|button| dialog_button_score(button, ACCEPT_DIALOG_BUTTONS))
            .filter(|button| dialog_button_has_label_match(button, ACCEPT_DIALOG_BUTTONS))
            .cloned()
    }

    enum DialogFileButtonSelection {
        Skip,
        Press(MacControlElementSummary),
    }

    fn select_dialog_file_button(
        dialogs: &[MacControlDialogSummary],
        request: &MacControlDialogRequest,
    ) -> Result<DialogFileButtonSelection, String> {
        let explicit = request
            .select_button
            .as_deref()
            .or(request.button_text.as_deref())
            .filter(|value| !value.is_empty());
        if let Some(query) = explicit {
            if query.eq_ignore_ascii_case("none") {
                return Ok(DialogFileButtonSelection::Skip);
            }
            if query.eq_ignore_ascii_case("default") {
                return select_dialog_default_button(dialogs)
                    .map(DialogFileButtonSelection::Press)
                    .ok_or_else(|| {
                        "No default accept-style dialog.file button matched.".to_string()
                    });
            }
            return dialogs
                .iter()
                .flat_map(|dialog| dialog.buttons.iter())
                .filter(|button| button.enabled != Some(false))
                .find(|button| element_label_matches(button, query))
                .cloned()
                .map(DialogFileButtonSelection::Press)
                .ok_or_else(|| format!("No dialog.file button matched explicit label {query:?}."));
        }
        select_dialog_default_button(dialogs)
            .map(DialogFileButtonSelection::Press)
            .ok_or_else(|| "No default accept-style dialog.file button matched.".to_string())
    }

    fn dialog_file_button_should_close(
        request: &MacControlDialogRequest,
        button: &MacControlElementSummary,
    ) -> bool {
        let explicit = request
            .select_button
            .as_deref()
            .or(request.button_text.as_deref())
            .filter(|value| !value.is_empty());
        match explicit {
            Some(value) if value.eq_ignore_ascii_case("none") => false,
            Some(value) if value.eq_ignore_ascii_case("default") => true,
            Some(_) => dialog_button_has_label_match(button, ACCEPT_DIALOG_BUTTONS),
            None => true,
        }
    }

    fn verify_dialog_file_closed(request: &MacControlDialogRequest, warnings: &mut Vec<String>) {
        thread::sleep(Duration::from_millis(250));
        match capture_ax_snapshot(MacControlSnapshotRequest {
            include_screenshot: false,
            max_elements: request.max_elements,
            max_depth: request.max_depth,
            ..Default::default()
        }) {
            Ok(snapshot) => {
                if !dialog_summaries(&snapshot, &request.target).is_empty() {
                    warnings.push(
                        "dialog.file clicked an accept-style button, but a dialog/sheet is still visible; the path/name may need another confirmation or validation failed."
                            .to_string(),
                    );
                }
            }
            Err(error) => warnings.push(format!(
                "dialog.file could not verify whether the dialog closed: {error}"
            )),
        }
    }

    fn select_dialog_field(
        dialogs: &[MacControlDialogSummary],
        request: &MacControlDialogRequest,
    ) -> Option<MacControlElementSummary> {
        let fields = dialogs
            .iter()
            .flat_map(|dialog| dialog.fields.iter())
            .filter(|field| field.enabled != Some(false))
            .collect::<Vec<_>>();
        if let Some(element_id) = request
            .target
            .element_id
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            return fields
                .iter()
                .find(|field| field.id == element_id)
                .map(|field| (*field).clone());
        }
        if let Some(index) = request.field_index {
            return fields.get(index).map(|field| (*field).clone());
        }
        if let Some(query) = request.field.as_deref().filter(|value| !value.is_empty()) {
            return fields
                .iter()
                .find(|field| {
                    field.id == query
                        || contains_ci(field.label.as_deref(), Some(query))
                        || contains_ci(field.value.as_deref(), Some(query))
                })
                .map(|field| (*field).clone());
        }
        fields
            .iter()
            .find(|field| field.focused)
            .copied()
            .or_else(|| fields.first().copied())
            .cloned()
    }

    fn handle_file_dialog(
        request: &MacControlDialogRequest,
        warnings: &mut Vec<String>,
    ) -> Result<MacControlDialogFileResult, String> {
        if request.ensure_expanded {
            warnings.push(
                "dialog.file ensureExpanded is best-effort in this bridge; using Go to Folder navigation when a path is provided."
                    .to_string(),
            );
        }
        let mut path_navigation = None;
        let mut name_field = None;
        if let Some(path) = request
            .file_path
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            post_hotkey(&["cmd".to_string(), "shift".to_string(), "g".to_string()])?;
            thread::sleep(Duration::from_millis(180));
            let paste_execution = paste_text_via_clipboard(path)?;
            post_key(key_code_for("enter").unwrap_or(36), 0)?;
            thread::sleep(Duration::from_millis(350));
            path_navigation = Some(format!("GoToFolder({paste_execution})"));
        }
        if let Some(name) = request
            .file_name
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            let snapshot = capture_ax_snapshot(MacControlSnapshotRequest {
                include_screenshot: false,
                max_elements: request.max_elements,
                max_depth: request.max_depth,
                ..Default::default()
            })?;
            let dialogs = dialog_summaries(&snapshot, &request.target);
            let field = select_dialog_field(&dialogs, request)
                .ok_or_else(|| "dialog.file could not find a filename text field.".to_string())?;
            let element =
                resolve_element_by_summary(&field, request.max_elements, request.max_depth)?;
            let element_ref = element.as_ptr() as AXUIElementRef;
            let set_name_execution = match set_ax_string(element_ref, "AXValue", name) {
                Ok(()) => "AXSetFilename".to_string(),
                Err(error) => replace_text_via_clipboard(
                    element_ref,
                    &field,
                    name,
                    "dialog.file filename fallback",
                    Some(error),
                )?,
            };
            name_field = Some(field);
            path_navigation = Some(match path_navigation {
                Some(existing) => format!("{existing}+{set_name_execution}"),
                None => set_name_execution,
            });
        }
        let requested_button = request
            .select_button
            .clone()
            .or_else(|| request.button_text.clone());
        Ok(MacControlDialogFileResult {
            path: request.file_path.clone(),
            name: request.file_name.clone(),
            requested_button,
            selected_button: None,
            name_field,
            path_navigation,
        })
    }

    const ACCEPT_DIALOG_BUTTONS: &[&str] = &[
        "ok", "open", "save", "choose", "select", "allow", "continue", "done", "yes", "replace",
        "好", "确定", "打開", "打开", "儲存", "保存", "选择", "選擇", "允许", "允許", "继续",
        "繼續", "完成", "是",
    ];
    const DISMISS_DIALOG_BUTTONS: &[&str] = &[
        "cancel",
        "close",
        "don't save",
        "dont save",
        "no",
        "not now",
        "later",
        "取消",
        "关闭",
        "關閉",
        "不保存",
        "否",
        "以后",
        "稍後",
    ];

    fn element_label_matches(element: &MacControlElementSummary, query: &str) -> bool {
        contains_ci(element.label.as_deref(), Some(query))
            || contains_ci(element.value.as_deref(), Some(query))
    }

    fn element_display_text(element: &MacControlElementSummary) -> Option<String> {
        element
            .label
            .clone()
            .or_else(|| element.value.clone())
            .filter(|value| !value.trim().is_empty())
    }

    fn dialog_button_has_label_match(
        element: &MacControlElementSummary,
        patterns: &[&str],
    ) -> bool {
        patterns
            .iter()
            .any(|pattern| element_label_matches(element, pattern))
    }

    fn dialog_button_score(element: &MacControlElementSummary, patterns: &[&str]) -> u8 {
        let mut score = 0;
        if element.enabled == Some(true) {
            score += 1;
        }
        for (idx, pattern) in patterns.iter().enumerate() {
            if element_label_matches(element, pattern) {
                score += (patterns.len().saturating_sub(idx).min(20)) as u8 + 5;
                break;
            }
        }
        score
    }

    fn press_dialog_button(
        element: AXUIElementRef,
        summary: &MacControlElementSummary,
    ) -> Result<(), String> {
        press_ax_or_click_center(element, summary, "dialog button").map(|_| ())
    }

    fn press_ax_or_click_center(
        element: AXUIElementRef,
        summary: &MacControlElementSummary,
        label: &str,
    ) -> Result<String, String> {
        match perform_ax_action(element, "AXPress") {
            Ok(()) => Ok("AXPress".to_string()),
            Err(ax_error) => {
                let point = point_for_element(summary, label).map_err(|bounds_error| {
                    format!(
                        "AXPress failed: {ax_error}; CGEvent fallback unavailable: {bounds_error}"
                    )
                })?;
                post_mouse_click(point, MouseButton::Left).map_err(|click_error| {
                    format!("AXPress failed: {ax_error}; CGEvent fallback failed: {click_error}")
                })?;
                Ok(format!(
                    "AXPressFailed+CGEventFallback({})",
                    truncate_for_error(&ax_error, 96)
                ))
            }
        }
    }

    fn dialog_op_name(op: MacControlDialogOp) -> &'static str {
        match op {
            MacControlDialogOp::Inspect => "inspect",
            MacControlDialogOp::List => "list",
            MacControlDialogOp::Click => "click",
            MacControlDialogOp::Input => "input",
            MacControlDialogOp::File => "file",
            MacControlDialogOp::Accept => "accept",
            MacControlDialogOp::Dismiss => "dismiss",
        }
    }

    fn app_matches_request(app: &MacControlRunningApp, request: &MacControlAppsRequest) -> bool {
        if request.pid.is_some_and(|pid| app.pid != pid) {
            return false;
        }
        if !contains_ci(app.bundle_id.as_deref(), request.bundle_id.as_deref()) {
            return false;
        }
        if let Some(app_name) = request
            .app_name
            .as_deref()
            .filter(|app_name| !app_name.is_empty())
        {
            return app_name_matches_values(
                request.app_name_match,
                app_name,
                [
                    app.name.as_deref(),
                    app.bundle_id
                        .as_deref()
                        .and_then(|bundle_id| bundle_id.rsplit('.').next()),
                    app.bundle_id.as_deref(),
                ],
            );
        }
        true
    }

    fn running_app_matches_request(
        app: &NSRunningApplication,
        request: &MacControlAppsRequest,
    ) -> bool {
        if request
            .pid
            .is_some_and(|pid| app.processIdentifier() != pid)
        {
            return false;
        }
        let bundle_id = app.bundleIdentifier().as_deref().map(ToString::to_string);
        if !contains_ci(bundle_id.as_deref(), request.bundle_id.as_deref()) {
            return false;
        }
        if let Some(app_name) = request
            .app_name
            .as_deref()
            .filter(|app_name| !app_name.is_empty())
        {
            let localized_name = app.localizedName().as_deref().map(ToString::to_string);
            let bundle_component = app
                .bundleURL()
                .and_then(|url| url.lastPathComponent())
                .as_deref()
                .map(ToString::to_string);
            let executable_component = app
                .executableURL()
                .and_then(|url| url.lastPathComponent())
                .as_deref()
                .map(ToString::to_string);
            return app_name_matches_values(
                request.app_name_match,
                app_name,
                [
                    localized_name.as_deref(),
                    bundle_id
                        .as_deref()
                        .and_then(|bundle_id| bundle_id.rsplit('.').next()),
                    bundle_component
                        .as_deref()
                        .map(|name| name.trim_end_matches(".app")),
                    executable_component.as_deref(),
                    bundle_id.as_deref(),
                ],
            );
        }
        true
    }

    fn app_name_matches_values<'a>(
        strategy: MacControlAppNameMatch,
        query: &str,
        values: impl IntoIterator<Item = Option<&'a str>>,
    ) -> bool {
        values
            .into_iter()
            .flatten()
            .any(|value| app_name_value_matches(strategy, value, query))
    }

    fn app_name_value_matches(strategy: MacControlAppNameMatch, value: &str, query: &str) -> bool {
        match strategy {
            MacControlAppNameMatch::Exact => {
                value.eq_ignore_ascii_case(query)
                    || normalize_app_token(value) == normalize_app_token(query)
            }
            MacControlAppNameMatch::Contains => {
                contains_ci(Some(value), Some(query)) || {
                    let value = normalize_app_token(value);
                    let query = normalize_app_token(query);
                    !value.is_empty() && !query.is_empty() && value.contains(&query)
                }
            }
        }
    }

    fn normalize_app_token(value: &str) -> String {
        value
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .map(|ch| ch.to_ascii_lowercase())
            .collect()
    }

    fn installed_apps_for_request(
        request: &MacControlAppsRequest,
        running: &[Retained<NSRunningApplication>],
    ) -> Vec<MacControlInstalledApp> {
        let mut apps = Vec::new();
        for path in discover_installed_app_paths() {
            if let Some(app) = installed_app_from_bundle_path(&path, running) {
                merge_installed_app(&mut apps, app);
            }
        }
        for app in running {
            merge_installed_app(&mut apps, installed_app_from_running(app));
        }
        apps.sort_by(|left, right| {
            left.name
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .cmp(
                    &right
                        .name
                        .as_deref()
                        .unwrap_or_default()
                        .to_ascii_lowercase(),
                )
                .then_with(|| {
                    left.bundle_id
                        .as_deref()
                        .unwrap_or_default()
                        .cmp(right.bundle_id.as_deref().unwrap_or_default())
                })
        });
        apps.into_iter()
            .filter(|app| installed_app_matches_request(app, request))
            .take(request.limit)
            .collect()
    }

    fn discover_installed_app_paths() -> Vec<PathBuf> {
        let mut paths = BTreeSet::new();
        if let Ok(output) = Command::new("/usr/bin/mdfind")
            .arg("kMDItemContentType == 'com.apple.application-bundle'")
            .output()
        {
            if output.status.success() {
                for line in String::from_utf8_lossy(&output.stdout).lines() {
                    let path = PathBuf::from(line.trim());
                    if path.extension().is_some_and(|ext| ext == "app") {
                        paths.insert(path);
                    }
                }
            }
        }
        if paths.is_empty() {
            for root in common_application_roots() {
                scan_app_paths(&root, 4, &mut paths);
            }
        }
        paths.into_iter().collect()
    }

    fn common_application_roots() -> Vec<PathBuf> {
        let mut roots = vec![
            PathBuf::from("/Applications"),
            PathBuf::from("/System/Applications"),
            PathBuf::from("/System/Applications/Utilities"),
        ];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join("Applications"));
        }
        roots
    }

    fn scan_app_paths(root: &Path, depth: usize, out: &mut BTreeSet<PathBuf>) {
        if depth == 0 || !root.is_dir() {
            return;
        }
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "app") {
                out.insert(path);
                continue;
            }
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                scan_app_paths(&path, depth - 1, out);
            }
        }
    }

    fn installed_app_from_bundle_path(
        path: &Path,
        running: &[Retained<NSRunningApplication>],
    ) -> Option<MacControlInstalledApp> {
        let path = path.to_string_lossy().to_string();
        let bundle = NSBundle::bundleWithPath(&NSString::from_str(&path));
        let bundle_id = bundle
            .as_deref()
            .and_then(|bundle| bundle.bundleIdentifier())
            .as_deref()
            .map(ToString::to_string);
        let executable_path = bundle
            .as_deref()
            .and_then(|bundle| bundle.executablePath())
            .as_deref()
            .map(ToString::to_string);
        let running_app =
            running_app_for_installed(bundle_id.as_deref(), Some(path.as_str()), running);
        let running_summary = running_app.as_deref().map(running_app_summary);
        Some(MacControlInstalledApp {
            name: running_summary
                .as_ref()
                .and_then(|app| app.name.clone())
                .or_else(|| app_bundle_name(Path::new(&path))),
            bundle_id: bundle_id.or_else(|| {
                running_summary
                    .as_ref()
                    .and_then(|app| app.bundle_id.clone())
            }),
            path: Some(path),
            executable_path,
            running: running_summary.is_some(),
            pid: running_summary.as_ref().map(|app| app.pid),
            active: running_summary.as_ref().is_some_and(|app| app.active),
            hidden: running_summary.as_ref().is_some_and(|app| app.hidden),
            activation_policy: running_summary
                .as_ref()
                .map(|app| app.activation_policy.clone()),
        })
    }

    fn installed_app_from_running(app: &NSRunningApplication) -> MacControlInstalledApp {
        let summary = running_app_summary(app);
        MacControlInstalledApp {
            name: summary.name,
            bundle_id: summary.bundle_id,
            path: app
                .bundleURL()
                .and_then(|url| url.path())
                .as_deref()
                .map(ToString::to_string),
            executable_path: app
                .executableURL()
                .and_then(|url| url.path())
                .as_deref()
                .map(ToString::to_string),
            running: true,
            pid: Some(summary.pid),
            active: summary.active,
            hidden: summary.hidden,
            activation_policy: Some(summary.activation_policy),
        }
    }

    fn running_app_for_installed<'a>(
        bundle_id: Option<&str>,
        path: Option<&str>,
        running: &'a [Retained<NSRunningApplication>],
    ) -> Option<&'a NSRunningApplication> {
        for app in running {
            let app: &NSRunningApplication = app.as_ref();
            let matches = bundle_id.is_some_and(|bundle_id| {
                app.bundleIdentifier()
                    .as_deref()
                    .map(ToString::to_string)
                    .as_deref()
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(bundle_id))
            }) || path.is_some_and(|path| {
                app.bundleURL()
                    .and_then(|url| url.path())
                    .as_deref()
                    .map(ToString::to_string)
                    .as_deref()
                    .is_some_and(|actual| actual == path)
            });
            if matches {
                return Some(app);
            }
        }
        None
    }

    fn merge_installed_app(apps: &mut Vec<MacControlInstalledApp>, app: MacControlInstalledApp) {
        if let Some(existing) = apps
            .iter_mut()
            .find(|existing| installed_app_same(existing, &app))
        {
            if !existing.running && app.running {
                existing.running = true;
                existing.pid = app.pid;
                existing.active = app.active;
                existing.hidden = app.hidden;
                existing.activation_policy = app.activation_policy;
                if existing.name.is_none() {
                    existing.name = app.name;
                }
                if existing.executable_path.is_none() {
                    existing.executable_path = app.executable_path;
                }
            }
            return;
        }
        apps.push(app);
    }

    fn installed_app_same(left: &MacControlInstalledApp, right: &MacControlInstalledApp) -> bool {
        left.bundle_id
            .as_deref()
            .zip(right.bundle_id.as_deref())
            .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
            || left
                .path
                .as_deref()
                .zip(right.path.as_deref())
                .is_some_and(|(left, right)| left == right)
    }

    fn installed_app_matches_request(
        app: &MacControlInstalledApp,
        request: &MacControlAppsRequest,
    ) -> bool {
        if request.pid.is_some_and(|pid| app.pid != Some(pid)) {
            return false;
        }
        if !contains_ci(app.bundle_id.as_deref(), request.bundle_id.as_deref()) {
            return false;
        }
        if let Some(app_name) = request
            .app_name
            .as_deref()
            .filter(|app_name| !app_name.is_empty())
        {
            let path_name = app
                .path
                .as_deref()
                .and_then(|path| app_bundle_name(Path::new(path)));
            let executable_name = app
                .executable_path
                .as_deref()
                .and_then(|path| file_name(Path::new(path)));
            return app_name_matches_values(
                request.app_name_match,
                app_name,
                [
                    app.name.as_deref(),
                    app.bundle_id
                        .as_deref()
                        .and_then(|bundle_id| bundle_id.rsplit('.').next()),
                    path_name.as_deref(),
                    executable_name.as_deref(),
                    app.bundle_id.as_deref(),
                ],
            );
        }
        true
    }

    fn app_bundle_name(path: &Path) -> Option<String> {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.trim_end_matches(".app").to_string())
            .filter(|name| !name.is_empty())
    }

    fn file_name(path: &Path) -> Option<String> {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(ToString::to_string)
            .filter(|name| !name.is_empty())
    }

    fn launch_app(
        request: &MacControlAppsRequest,
    ) -> Result<Retained<NSRunningApplication>, String> {
        let workspace = NSWorkspace::sharedWorkspace();
        if let Some(bundle_id) = request.bundle_id.as_deref() {
            let bundle_id_string = NSString::from_str(bundle_id);
            let url = workspace
                .URLForApplicationWithBundleIdentifier(&bundle_id_string)
                .ok_or_else(|| format!("No installed macOS app has bundleId '{bundle_id}'."))?;
            let ok = workspace.openURL(&url);
            if !ok {
                return Err(format!("macOS refused to open app bundle '{bundle_id}'."));
            }
            return wait_for_launched_app(request);
        }
        if let Some(app_name) = request.app_name.as_deref() {
            let app_name = NSString::from_str(app_name);
            #[allow(deprecated)]
            let ok = workspace.launchApplication(&app_name);
            if !ok {
                return Err("macOS refused to launch the requested app name.".to_string());
            }
            return wait_for_launched_app(request);
        }
        Err("apps.launch requires bundleId or appName.".to_string())
    }

    fn wait_for_launched_app(
        request: &MacControlAppsRequest,
    ) -> Result<Retained<NSRunningApplication>, String> {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(5) {
            let running = NSWorkspace::sharedWorkspace()
                .runningApplications()
                .to_vec();
            if let Some(app) = find_running_app_for_request(request, &running, &[]) {
                return Ok(app);
            }
            thread::sleep(Duration::from_millis(100));
        }
        Err("Timed out waiting for launched macOS app to appear.".to_string())
    }

    fn activate_running_app(app: &NSRunningApplication) -> Result<(), String> {
        let ok = app.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
        if !ok {
            return Err("macOS refused the app activation request.".to_string());
        }
        let pid = app.processIdentifier();
        let bundle_id = app.bundleIdentifier().as_deref().map(ToString::to_string);
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(2) {
            if focused_app_summary().is_some_and(|frontmost| {
                frontmost.pid == pid
                    || bundle_id
                        .as_deref()
                        .is_some_and(|bundle_id| frontmost.bundle_id.as_deref() == Some(bundle_id))
            }) {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }
        Err("Timed out waiting for macOS to focus the activated app.".to_string())
    }

    fn quit_running_app(
        app: &NSRunningApplication,
        summary: &MacControlRunningApp,
    ) -> Result<String, String> {
        if app.terminate() {
            return Ok("NSRunningApplication.terminate".to_string());
        }
        if let Some(bundle_id) = summary.bundle_id.as_deref() {
            let script = format!(
                "tell application id {} to quit",
                apple_script_string(bundle_id)
            );
            run_osascript(&script)?;
            return Ok("AppleEvents.quitByBundleId".to_string());
        }
        if let Some(name) = summary.name.as_deref() {
            let script = format!("tell application {} to quit", apple_script_string(name));
            run_osascript(&script)?;
            return Ok("AppleEvents.quitByName".to_string());
        }
        Err("macOS refused to quit the app and no Apple Events target was available.".to_string())
    }

    fn find_running_app_for_request(
        request: &MacControlAppsRequest,
        running: &[Retained<NSRunningApplication>],
        candidates: &[MacControlRunningApp],
    ) -> Option<Retained<NSRunningApplication>> {
        if let Some(pid) = request.pid {
            return NSRunningApplication::runningApplicationWithProcessIdentifier(pid);
        }

        if let Some(bundle_id) = request
            .bundle_id
            .as_deref()
            .filter(|bundle_id| !bundle_id.is_empty())
        {
            if let Some(app) = running_apps_with_bundle_id(bundle_id)
                .iter()
                .find(|app| running_app_matches_request(app, request))
            {
                return Some(app.clone());
            }
        }

        if let Some(app) = running
            .iter()
            .find(|app| running_app_matches_request(app, request))
        {
            return Some(app.clone());
        }

        candidates
            .iter()
            .find(|app| app_matches_request(app, request))
            .and_then(|app| NSRunningApplication::runningApplicationWithProcessIdentifier(app.pid))
    }

    fn focused_app_element() -> Result<CfOwned, String> {
        let system = unsafe { AXUIElementCreateSystemWide() };
        let system = CfOwned::new(system as CFTypeRef)
            .ok_or_else(|| "Unable to create the system Accessibility element.".to_string())?;
        copy_attribute(system.as_ptr() as AXUIElementRef, "AXFocusedApplication")
            .ok_or_else(|| "Unable to read focused macOS application.".to_string())
    }

    fn focused_element() -> Option<(CfOwned, MacControlElementSummary)> {
        let system = unsafe { AXUIElementCreateSystemWide() };
        let system = CfOwned::new(system as CFTypeRef)?;
        let element = copy_attribute(system.as_ptr() as AXUIElementRef, "AXFocusedUIElement")?;
        let summary = element_summary(element.as_ptr() as AXUIElementRef, None, 1);
        Some((element, summary))
    }

    fn resolve_window(
        request: &MacControlWindowsRequest,
    ) -> Result<(CfOwned, MacControlWindowSummary), String> {
        let candidates = window_candidate_apps(request)?;
        let mut matches = Vec::new();
        for candidate in candidates {
            let Some(windows) =
                copy_attribute(candidate.element.as_ptr() as AXUIElementRef, "AXWindows")
            else {
                continue;
            };
            for (idx, window_ref) in cf_array_values(windows.as_ptr()).into_iter().enumerate() {
                let id = if candidate.all_scope_ids {
                    format!("win_{}_{}", candidate.pid, idx + 1)
                } else {
                    format!("win_{}", idx + 1)
                };
                let summary =
                    window_summary_for_app(window_ref as AXUIElementRef, &id, Some(candidate.pid));
                if window_matches_request(&summary, request) {
                    let retained = unsafe { CFRetain(window_ref as CFTypeRef) };
                    let window = CfOwned::new(retained)
                        .ok_or_else(|| "Unable to retain matched AX window.".to_string())?;
                    matches.push((window, summary));
                }
            }
        }
        match matches.len() {
            0 => Err("No macOS window matched the request.".to_string()),
            1 => Ok(matches.remove(0)),
            count => Err(format!(
                "{count} macOS windows matched the request; retry with a precise windowId or app target."
            )),
        }
    }

    struct WindowCandidateApp {
        element: CfOwned,
        pid: i32,
        all_scope_ids: bool,
    }

    fn window_candidate_apps(
        request: &MacControlWindowsRequest,
    ) -> Result<Vec<WindowCandidateApp>, String> {
        if let Some(pid) = window_id_all_scope_pid(request.window_id.as_deref()) {
            let element = app_element_for_pid(pid).ok_or_else(|| {
                format!("Unable to create Accessibility app element for pid {pid}.")
            })?;
            return Ok(vec![WindowCandidateApp {
                element,
                pid,
                all_scope_ids: true,
            }]);
        }

        if request.window_scope == MacControlWindowsScope::All
            || target_has_app_filter(&request.target)
        {
            return all_window_candidate_apps(&request.target);
        }

        let app = focused_app_element()?;
        let summary = app_summary(app.as_ptr() as AXUIElementRef);
        if !app_matches_target(&summary, &request.target) {
            return Err("Frontmost app did not match the windows target.".to_string());
        }
        Ok(vec![WindowCandidateApp {
            element: app,
            pid: summary.pid,
            all_scope_ids: false,
        }])
    }

    fn all_window_candidate_apps(
        target: &MacControlTargetQuery,
    ) -> Result<Vec<WindowCandidateApp>, String> {
        let workspace = NSWorkspace::sharedWorkspace();
        let mut seen = BTreeSet::new();
        let mut candidates = Vec::new();
        for app in workspace.runningApplications().to_vec() {
            let summary = running_app_summary(&app);
            if !seen.insert(summary.pid) {
                continue;
            }
            if target_has_app_filter(target)
                && !running_app_summary_matches_target(&summary, target)
            {
                continue;
            }
            if let Some(element) = app_element_for_pid(summary.pid) {
                candidates.push(WindowCandidateApp {
                    element,
                    pid: summary.pid,
                    all_scope_ids: true,
                });
            }
        }
        if candidates.is_empty() {
            Err("No running macOS app matched the windows target.".to_string())
        } else {
            Ok(candidates)
        }
    }

    fn window_matches_request(
        window: &MacControlWindowSummary,
        request: &MacControlWindowsRequest,
    ) -> bool {
        if request
            .window_id
            .as_deref()
            .filter(|query| !query.is_empty())
            .is_some_and(|query| !window_id_matches(query, &window.id))
        {
            return false;
        }
        window_title_matches(window.title.as_deref(), &request.target)
    }

    fn window_id_matches(query: &str, actual: &str) -> bool {
        if query == actual {
            return true;
        }
        let Some(query_idx) = legacy_window_id_index(query) else {
            return false;
        };
        all_scope_window_id_parts(actual).is_some_and(|(_, actual_idx)| actual_idx == query_idx)
    }

    fn legacy_window_id_index(value: &str) -> Option<usize> {
        let mut parts = value.split('_');
        match (parts.next(), parts.next(), parts.next()) {
            (Some("win"), Some(idx), None) => idx.parse::<usize>().ok(),
            _ => None,
        }
    }

    fn window_id_all_scope_pid(value: Option<&str>) -> Option<i32> {
        value.and_then(|value| all_scope_window_id_parts(value).map(|(pid, _)| pid))
    }

    fn all_scope_window_id_parts(value: &str) -> Option<(i32, usize)> {
        let mut parts = value.split('_');
        match (parts.next(), parts.next(), parts.next(), parts.next()) {
            (Some("win"), Some(pid), Some(idx), None) => {
                Some((pid.parse::<i32>().ok()?, idx.parse::<usize>().ok()?))
            }
            _ => None,
        }
    }

    fn resolve_element(
        target: &MacControlTargetQuery,
        max_elements: usize,
        max_depth: usize,
        op_label: &str,
    ) -> Result<(CfOwned, MacControlElementSummary, MacControlSnapshot), String> {
        resolve_target_element(
            target,
            max_elements,
            max_depth,
            op_label,
            ElementResolveMode::Any,
            "AX element",
        )
    }

    fn resolve_type_element(
        target: &MacControlTargetQuery,
        max_elements: usize,
        max_depth: usize,
        op_label: &str,
    ) -> Result<(CfOwned, MacControlElementSummary, MacControlSnapshot), String> {
        resolve_target_element(
            target,
            max_elements,
            max_depth,
            op_label,
            ElementResolveMode::TextInput,
            "text input element",
        )
    }

    #[derive(Clone, Copy)]
    enum ElementResolveMode {
        Any,
        TextInput,
    }

    fn resolve_target_element(
        target: &MacControlTargetQuery,
        max_elements: usize,
        max_depth: usize,
        op_label: &str,
        mode: ElementResolveMode,
        target_label: &str,
    ) -> Result<(CfOwned, MacControlElementSummary, MacControlSnapshot), String> {
        let snapshot = capture_ax_snapshot(MacControlSnapshotRequest {
            include_screenshot: false,
            max_elements,
            max_depth,
            ..Default::default()
        })?;
        if !frontmost_app_matches_act_target(&snapshot, target) {
            return Err(format!(
                "Frontmost app did not match the {op_label} target."
            ));
        }
        let summary = if let Some(summary) =
            select_snapshot_anchored_element(target, &snapshot, op_label, mode, target_label)?
        {
            summary
        } else {
            let candidates = snapshot
                .elements
                .iter()
                .filter(|element| element_matches_for_mode(element, target, &snapshot, mode))
                .map(|element| ScoredElementMatch {
                    score: element_target_score_for_mode(element, target, mode),
                    summary: element.clone(),
                })
                .collect();
            select_element_match(candidates, target, op_label, target_label)?
        };
        let element = resolve_element_by_summary(&summary, max_elements, max_depth)?;
        Ok((element, summary, snapshot))
    }

    fn element_matches_for_mode(
        element: &MacControlElementSummary,
        target: &MacControlTargetQuery,
        snapshot: &MacControlSnapshot,
        mode: ElementResolveMode,
    ) -> bool {
        match mode {
            ElementResolveMode::Any => element_matches_query(element, target, snapshot),
            ElementResolveMode::TextInput => text_element_matches_query(element, target, snapshot),
        }
    }

    fn element_target_score_for_mode(
        element: &MacControlElementSummary,
        target: &MacControlTargetQuery,
        mode: ElementResolveMode,
    ) -> u8 {
        match mode {
            ElementResolveMode::Any => element_target_score(element, target),
            ElementResolveMode::TextInput => type_target_score(element, target),
        }
    }

    fn select_snapshot_anchored_element(
        target: &MacControlTargetQuery,
        snapshot: &MacControlSnapshot,
        op_label: &str,
        mode: ElementResolveMode,
        target_label: &str,
    ) -> Result<Option<MacControlElementSummary>, String> {
        let Some(snapshot_id) = target
            .snapshot_id
            .as_deref()
            .filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };
        let Some(element_id) = target
            .element_id
            .as_deref()
            .filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };
        let previous = ha_core::mac_control::cached_snapshot(snapshot_id).ok_or_else(|| {
            format!(
                "{op_label} target.snapshotId '{snapshot_id}' was not found or expired; take a fresh snapshot or visual.observe before acting."
            )
        })?;
        let expected = previous
            .elements
            .iter()
            .find(|element| element.id == element_id)
            .ok_or_else(|| {
                format!(
                    "{op_label} target.elementId '{element_id}' was not found in target.snapshotId '{snapshot_id}'; retry with a fresh snapshot."
                )
            })?;
        ensure_snapshot_anchor_app_matches(target, snapshot, &previous, op_label)?;
        let target_without_ids = target_without_snapshot_ids(target);
        let mut candidates = snapshot
            .elements
            .iter()
            .filter(|element| {
                element_matches_for_mode(element, &target_without_ids, snapshot, mode)
            })
            .filter_map(|element| {
                anchored_element_score(element, expected, snapshot, &previous).map(|score| {
                    ScoredAnchoredElementMatch {
                        score,
                        summary: element.clone(),
                    }
                })
            })
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return Err(format!(
                "{op_label} target.elementId '{element_id}' from snapshotId '{snapshot_id}' no longer matched a stable {target_label}; take a fresh snapshot and retry."
            ));
        }
        candidates.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.summary.id.cmp(&right.summary.id))
        });
        let top_score = candidates[0].score;
        let equal_top_count = candidates
            .iter()
            .take_while(|candidate| candidate.score == top_score)
            .count();
        if equal_top_count > 1 {
            let preview = candidates
                .iter()
                .take(equal_top_count.min(5))
                .map(|candidate| element_candidate_hint(&candidate.summary))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(format!(
                "{equal_top_count} {target_label}s matched the {op_label} snapshot anchor equally; retry with a fresh snapshot plus target.windowTitle, target.role, or more specific target.text. Candidates: {preview}"
            ));
        }
        Ok(Some(candidates.remove(0).summary))
    }

    fn ensure_snapshot_anchor_app_matches(
        target: &MacControlTargetQuery,
        snapshot: &MacControlSnapshot,
        previous: &MacControlSnapshot,
        op_label: &str,
    ) -> Result<(), String> {
        if target_has_app_filter(target) {
            return Ok(());
        }
        let Some(previous_app) = previous.frontmost_app.as_ref() else {
            return Ok(());
        };
        let Some(current_app) = snapshot.frontmost_app.as_ref() else {
            return Err(format!(
                "{op_label} target.snapshotId '{}' had frontmost app '{}', but the current frontmost app could not be resolved; observe again before acting.",
                previous.snapshot_id,
                app_anchor_label(previous_app)
            ));
        };
        if app_summaries_match_anchor(current_app, previous_app) {
            return Ok(());
        }
        Err(format!(
            "{op_label} target.snapshotId '{}' was captured from '{}', but the current frontmost app is '{}'; observe again or pass an explicit target.bundleId/appName.",
            previous.snapshot_id,
            app_anchor_label(previous_app),
            app_anchor_label(current_app)
        ))
    }

    fn app_summaries_match_anchor(
        current: &MacControlAppSummary,
        previous: &MacControlAppSummary,
    ) -> bool {
        if current.pid == previous.pid {
            return true;
        }
        match (
            non_empty(current.bundle_id.as_deref()),
            non_empty(previous.bundle_id.as_deref()),
        ) {
            (Some(current_bundle), Some(previous_bundle))
                if current_bundle.eq_ignore_ascii_case(previous_bundle) =>
            {
                true
            }
            _ => match (
                non_empty(current.name.as_deref()),
                non_empty(previous.name.as_deref()),
            ) {
                (Some(current_name), Some(previous_name)) => {
                    current_name.eq_ignore_ascii_case(previous_name)
                }
                _ => false,
            },
        }
    }

    fn app_anchor_label(app: &MacControlAppSummary) -> String {
        if let Some(bundle_id) = non_empty(app.bundle_id.as_deref()) {
            format!(
                "{} ({bundle_id}, pid {})",
                app.name.as_deref().unwrap_or("unknown"),
                app.pid
            )
        } else if let Some(name) = non_empty(app.name.as_deref()) {
            format!("{name} (pid {})", app.pid)
        } else {
            format!("pid {}", app.pid)
        }
    }

    fn target_without_snapshot_ids(target: &MacControlTargetQuery) -> MacControlTargetQuery {
        let mut target = target.clone();
        target.element_id = None;
        target.snapshot_id = None;
        target
    }

    #[derive(Clone)]
    struct ScoredAnchoredElementMatch {
        score: u16,
        summary: MacControlElementSummary,
    }

    fn anchored_element_score(
        actual: &MacControlElementSummary,
        expected: &MacControlElementSummary,
        actual_snapshot: &MacControlSnapshot,
        expected_snapshot: &MacControlSnapshot,
    ) -> Option<u16> {
        let mut score = 0_u16;
        if let Some(expected_role) = non_empty(expected.role.as_deref()) {
            if !optional_eq_ci(actual.role.as_deref(), expected_role) {
                return None;
            }
            score += 35;
        }

        if let Some(expected_window_title) = element_window_title(expected, expected_snapshot) {
            let actual_window_title = element_window_title(actual, actual_snapshot)?;
            if !actual_window_title.eq_ignore_ascii_case(&expected_window_title) {
                return None;
            }
            score += 20;
        } else if actual.window_id.is_some()
            && expected.window_id.is_some()
            && actual.window_id == expected.window_id
        {
            score += 8;
        }

        if actual.id == expected.id {
            score += 10;
        }

        if let Some(expected_label) = non_empty(expected.label.as_deref()) {
            if !optional_eq_ci(actual.label.as_deref(), expected_label) {
                return None;
            }
            score += 45;
        } else if let Some(expected_value) = non_empty(expected.value.as_deref()) {
            if !is_text_input_element(expected) {
                if !optional_eq_ci(actual.value.as_deref(), expected_value) {
                    return None;
                }
                score += 35;
            } else if optional_eq_ci(actual.value.as_deref(), expected_value) {
                score += 8;
            }
        }

        let bounds_score = anchored_bounds_score(actual.bounds_points, expected.bounds_points);
        if bounds_score == 0
            && non_empty(expected.label.as_deref()).is_none()
            && non_empty(expected.value.as_deref()).is_none()
        {
            return None;
        }
        score += bounds_score;

        if expected.enabled.is_some() && actual.enabled == expected.enabled {
            score += 3;
        }
        score += shared_action_score(&actual.actions, &expected.actions);

        (score >= 45).then_some(score)
    }

    fn element_window_title(
        element: &MacControlElementSummary,
        snapshot: &MacControlSnapshot,
    ) -> Option<String> {
        let window_id = element.window_id.as_deref()?;
        snapshot
            .windows
            .iter()
            .find(|window| window.id == window_id)
            .and_then(|window| non_empty(window.title.as_deref()))
            .map(str::to_string)
    }

    fn anchored_bounds_score(
        actual: Option<MacControlBounds>,
        expected: Option<MacControlBounds>,
    ) -> u16 {
        let (Some(actual), Some(expected)) = (actual, expected) else {
            return 0;
        };
        let center_dx = (bounds_center_x(actual) - bounds_center_x(expected)).abs();
        let center_dy = (bounds_center_y(actual) - bounds_center_y(expected)).abs();
        let size_delta =
            (actual.width - expected.width).abs() + (actual.height - expected.height).abs();
        if center_dx <= 4.0 && center_dy <= 4.0 && size_delta <= 8.0 {
            25
        } else if center_dx <= 20.0 && center_dy <= 20.0 && size_delta <= 40.0 {
            18
        } else if center_dx <= 64.0 && center_dy <= 64.0 {
            8
        } else {
            0
        }
    }

    fn bounds_center_x(bounds: MacControlBounds) -> f64 {
        bounds.x + bounds.width / 2.0
    }

    fn bounds_center_y(bounds: MacControlBounds) -> f64 {
        bounds.y + bounds.height / 2.0
    }

    fn shared_action_score(actual: &[String], expected: &[String]) -> u16 {
        let shared = actual
            .iter()
            .filter(|actual_action| {
                expected
                    .iter()
                    .any(|expected_action| expected_action.eq_ignore_ascii_case(actual_action))
            })
            .count();
        shared.min(5) as u16
    }

    fn non_empty(value: Option<&str>) -> Option<&str> {
        value.and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then_some(trimmed)
        })
    }

    #[derive(Clone)]
    struct ScoredElementMatch {
        score: u8,
        summary: MacControlElementSummary,
    }

    fn select_element_match(
        mut candidates: Vec<ScoredElementMatch>,
        target: &MacControlTargetQuery,
        op_label: &str,
        target_label: &str,
    ) -> Result<MacControlElementSummary, String> {
        if candidates.is_empty() {
            return Err(format!("No {target_label} matched the {op_label} target."));
        }
        if target
            .element_id
            .as_deref()
            .is_some_and(|element_id| !element_id.is_empty())
        {
            return Ok(candidates.remove(0).summary);
        }
        candidates.sort_by(|left, right| right.score.cmp(&left.score));
        let top_score = candidates[0].score;
        let equal_top_count = candidates
            .iter()
            .take_while(|candidate| candidate.score == top_score)
            .count();
        if equal_top_count > 1 {
            let preview = candidates
                .iter()
                .take(equal_top_count.min(5))
                .map(|candidate| element_candidate_hint(&candidate.summary))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(format!(
                "{equal_top_count} {target_label}s matched the {op_label} target equally; retry with elementId from snapshot, target.windowTitle, target.role, or more specific target.text. Candidates: {preview}"
            ));
        }
        Ok(candidates.remove(0).summary)
    }

    fn element_candidate_hint(element: &MacControlElementSummary) -> String {
        let mut parts = vec![element.id.clone()];
        if let Some(role) = element.role.as_deref().filter(|value| !value.is_empty()) {
            parts.push(format!("role={role}"));
        }
        if let Some(label) = element.label.as_deref().filter(|value| !value.is_empty()) {
            parts.push(format!("label=\"{}\"", truncate_for_error(label, 48)));
        }
        if let Some(window_id) = element
            .window_id
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            parts.push(format!("windowId={window_id}"));
        }
        parts.join(" ")
    }

    fn truncate_for_error(value: &str, max_chars: usize) -> String {
        let mut chars = value.chars();
        let mut truncated = String::new();
        for _ in 0..max_chars {
            let Some(ch) = chars.next() else {
                return value.to_string();
            };
            truncated.push(ch);
        }
        if chars.next().is_some() {
            truncated.push_str("...");
        }
        truncated
    }

    fn frontmost_app_matches_act_target(
        snapshot: &MacControlSnapshot,
        target: &MacControlTargetQuery,
    ) -> bool {
        let Some(app) = snapshot.frontmost_app.as_ref() else {
            return target.app_name.is_none() && target.bundle_id.is_none();
        };
        if !contains_ci(app.name.as_deref(), target.app_name.as_deref()) {
            return false;
        }
        if let Some(bundle_id) = target
            .bundle_id
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            return contains_ci(app.bundle_id.as_deref(), Some(bundle_id));
        }
        true
    }

    fn app_matches_target(app: &MacControlAppSummary, target: &MacControlTargetQuery) -> bool {
        if !contains_ci(app.name.as_deref(), target.app_name.as_deref()) {
            return false;
        }
        if let Some(bundle_id) = target
            .bundle_id
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            return contains_ci(app.bundle_id.as_deref(), Some(bundle_id));
        }
        true
    }

    fn running_app_summary_matches_target(
        app: &MacControlRunningApp,
        target: &MacControlTargetQuery,
    ) -> bool {
        let app = MacControlAppSummary {
            pid: app.pid,
            bundle_id: app.bundle_id.clone(),
            name: app.name.clone(),
        };
        app_matches_target(&app, target)
    }

    fn target_has_app_filter(target: &MacControlTargetQuery) -> bool {
        target
            .app_name
            .as_deref()
            .is_some_and(|value| !value.is_empty())
            || target
                .bundle_id
                .as_deref()
                .is_some_and(|value| !value.is_empty())
    }

    fn text_element_matches_query(
        element: &MacControlElementSummary,
        target: &MacControlTargetQuery,
        snapshot: &MacControlSnapshot,
    ) -> bool {
        if !is_text_input_element(element) {
            return false;
        }
        if !target
            .element_id
            .as_deref()
            .filter(|query| !query.is_empty())
            .map(|query| element.id == query)
            .unwrap_or(true)
        {
            return false;
        }
        if !contains_ci(element.role.as_deref(), target.role.as_deref()) {
            return false;
        }
        if !target
            .text
            .as_deref()
            .filter(|query| !query.is_empty())
            .map(|query| {
                contains_ci(element.label.as_deref(), Some(query))
                    || contains_ci(element.value.as_deref(), Some(query))
            })
            .unwrap_or(true)
        {
            return false;
        }
        if target.enabled == Some(true) && element.enabled == Some(false) {
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

    fn type_target_score(element: &MacControlElementSummary, target: &MacControlTargetQuery) -> u8 {
        let mut score = 0;
        if element.focused {
            score += 8;
        }
        score += text_role_score(element);
        if target
            .element_id
            .as_deref()
            .is_some_and(|query| !query.is_empty() && query == element.id)
        {
            score += 4;
        }
        if element.enabled == Some(true) {
            score += 1;
        }
        score
    }

    fn text_role_score(element: &MacControlElementSummary) -> u8 {
        let Some(role) = element.role.as_deref().map(str::to_ascii_lowercase) else {
            return 0;
        };
        if role.contains("textarea") {
            4
        } else if role.contains("textfield") || role.contains("searchfield") {
            3
        } else if role.contains("combobox") {
            1
        } else {
            0
        }
    }

    fn is_text_input_element(element: &MacControlElementSummary) -> bool {
        let Some(role) = element.role.as_deref().map(str::to_ascii_lowercase) else {
            return false;
        };
        if role.contains("statictext") {
            return false;
        }
        role.contains("textarea")
            || role.contains("textfield")
            || role.contains("searchfield")
            || role.contains("combobox")
    }

    fn element_role_is_web_area(element: &MacControlElementSummary) -> bool {
        element
            .role
            .as_deref()
            .is_some_and(|role| role.eq_ignore_ascii_case("AXWebArea"))
    }

    fn focused_text_input_element(element: &MacControlElementSummary) -> bool {
        element.focused && is_text_input_element(element)
    }

    fn resolve_element_by_summary(
        expected: &MacControlElementSummary,
        max_elements: usize,
        max_depth: usize,
    ) -> Result<CfOwned, String> {
        let app = focused_app_element()?;
        let mut state = CaptureState {
            max_elements,
            max_depth,
            next_element_id: 1,
            elements: Vec::new(),
            truncated: false,
        };
        for root in focused_overlay_roots(app.as_ptr() as AXUIElementRef) {
            if let Some(element) = find_element_by_generated_summary(
                root.element.as_ptr() as AXUIElementRef,
                0,
                root.window_id.as_deref(),
                &mut state,
                expected,
            )? {
                return Ok(element);
            }
            if state.truncated {
                return Err("Matched AX element became stale before action.".to_string());
            }
        }
        if let Some(windows) = copy_attribute(app.as_ptr() as AXUIElementRef, "AXWindows") {
            for (idx, window_ref) in cf_array_values(windows.as_ptr()).into_iter().enumerate() {
                let window_id = format!("win_{}", idx + 1);
                if let Some(element) = find_element_by_generated_summary(
                    window_ref as AXUIElementRef,
                    0,
                    Some(&window_id),
                    &mut state,
                    expected,
                )? {
                    return Ok(element);
                }
            }
        }
        find_element_by_generated_summary(
            app.as_ptr() as AXUIElementRef,
            0,
            None,
            &mut state,
            expected,
        )
        .and_then(|element| {
            element.ok_or_else(|| "Matched AX element became stale before action.".to_string())
        })
    }

    fn find_element_by_generated_summary(
        element: AXUIElementRef,
        depth: usize,
        window_id: Option<&str>,
        state: &mut CaptureState,
        expected: &MacControlElementSummary,
    ) -> Result<Option<CfOwned>, String> {
        if state.elements.len() >= state.max_elements {
            state.truncated = true;
            return Ok(None);
        }
        let summary = element_summary(element, window_id, state.next_element_id);
        if should_include_element(&summary) {
            state.next_element_id += 1;
            state.elements.push(summary.clone());
            if summary.id == expected.id {
                ensure_element_fingerprint_matches(&summary, expected)?;
                let retained = unsafe { CFRetain(element as CFTypeRef) };
                return Ok(CfOwned::new(retained));
            }
            if state.elements.len() >= state.max_elements {
                state.truncated = true;
                return Ok(None);
            }
        }
        if depth >= state.max_depth {
            return Ok(None);
        }
        let children = copy_attribute(element, "AXChildren")
            .or_else(|| copy_attribute(element, "AXVisibleChildren"));
        let Some(children) = children else {
            return Ok(None);
        };
        for child_ref in cf_array_values(children.as_ptr()) {
            if let Some(found) = find_element_by_generated_summary(
                child_ref as AXUIElementRef,
                depth + 1,
                window_id,
                state,
                expected,
            )? {
                return Ok(Some(found));
            }
            if state.truncated {
                return Ok(None);
            }
        }
        Ok(None)
    }

    fn ensure_element_fingerprint_matches(
        actual: &MacControlElementSummary,
        expected: &MacControlElementSummary,
    ) -> Result<(), String> {
        if actual.window_id != expected.window_id
            || actual.role != expected.role
            || actual.label != expected.label
            || actual.value != expected.value
            || !bounds_match(actual.bounds_points, expected.bounds_points)
        {
            return Err(
                "Matched AX element id now points to different UI state; retry with a fresh snapshot."
                    .to_string(),
            );
        }
        Ok(())
    }

    fn bounds_match(actual: Option<MacControlBounds>, expected: Option<MacControlBounds>) -> bool {
        match (actual, expected) {
            (None, None) => true,
            (Some(actual), Some(expected)) => {
                let tolerance = 4.0;
                (actual.x - expected.x).abs() <= tolerance
                    && (actual.y - expected.y).abs() <= tolerance
                    && (actual.width - expected.width).abs() <= tolerance
                    && (actual.height - expected.height).abs() <= tolerance
            }
            _ => false,
        }
    }

    fn element_matches_query(
        element: &MacControlElementSummary,
        target: &MacControlTargetQuery,
        snapshot: &MacControlSnapshot,
    ) -> bool {
        if !target
            .element_id
            .as_deref()
            .filter(|query| !query.is_empty())
            .map(|query| element.id == query)
            .unwrap_or(true)
        {
            return false;
        }
        if !contains_ci(element.role.as_deref(), target.role.as_deref()) {
            return false;
        }
        if !target
            .text
            .as_deref()
            .filter(|query| !query.is_empty())
            .map(|query| {
                contains_ci(element.label.as_deref(), Some(query))
                    || contains_ci(element.value.as_deref(), Some(query))
            })
            .unwrap_or(true)
        {
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

    fn element_target_score(
        element: &MacControlElementSummary,
        target: &MacControlTargetQuery,
    ) -> u8 {
        let mut score = 0;
        if target
            .element_id
            .as_deref()
            .is_some_and(|query| !query.is_empty() && query == element.id)
        {
            score += 80;
        }
        if element.focused {
            score += 12;
        }
        if element.enabled == Some(true) {
            score += 8;
        }
        if element.actions.iter().any(|action| action == "AXPress") {
            score += 6;
        }
        if element.bounds_points.is_some() {
            score += 2;
        }
        if let Some(query) = target.text.as_deref().filter(|query| !query.is_empty()) {
            if optional_eq_ci(element.label.as_deref(), query)
                || optional_eq_ci(element.value.as_deref(), query)
            {
                score += 10;
            }
        }
        score
    }

    fn target_query_is_empty(target: &MacControlTargetQuery) -> bool {
        target.app_name.as_deref().is_none_or(str::is_empty)
            && target.bundle_id.as_deref().is_none_or(str::is_empty)
            && target.window_title.as_deref().is_none_or(str::is_empty)
            && target.element_id.as_deref().is_none_or(str::is_empty)
            && target.text.as_deref().is_none_or(str::is_empty)
            && target.role.as_deref().is_none_or(str::is_empty)
            && target.enabled.is_none()
            && target.focused.is_none()
    }

    fn running_apps_with_bundle_id(bundle_id: &str) -> Vec<Retained<NSRunningApplication>> {
        let bundle_id = NSString::from_str(bundle_id);
        NSRunningApplication::runningApplicationsWithBundleIdentifier(&bundle_id).to_vec()
    }

    fn perform_ax_action(element: AXUIElementRef, action: &str) -> Result<(), String> {
        let action = cf_string(action)?;
        let err = unsafe { AXUIElementPerformAction(element, action.as_ptr() as CFStringRef) };
        if err == AX_ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("AX action failed with error {err}."))
        }
    }

    fn set_ax_string(element: AXUIElementRef, attribute: &str, value: &str) -> Result<(), String> {
        let attribute = cf_string(attribute)?;
        let value = cf_string(value)?;
        set_ax_value(element, attribute.as_ptr() as CFStringRef, value.as_ptr())
    }

    fn set_ax_bool(element: AXUIElementRef, attribute: &str, value: bool) -> Result<(), String> {
        let attribute = cf_string(attribute)?;
        let value = if value {
            unsafe { kCFBooleanTrue }
        } else {
            return Err("Setting false AX booleans is not supported in Phase 3.".to_string());
        };
        set_ax_value(element, attribute.as_ptr() as CFStringRef, value)
    }

    fn set_ax_point(
        element: AXUIElementRef,
        attribute: &str,
        point: CGPoint,
    ) -> Result<(), String> {
        let attribute = cf_string(attribute)?;
        let value =
            unsafe { AXValueCreate(K_AXVALUE_CGPOINT_TYPE, &point as *const _ as *const c_void) };
        let value = CfOwned::new(value as CFTypeRef)
            .ok_or_else(|| "AXValueCreate(point) returned null.".to_string())?;
        set_ax_value(element, attribute.as_ptr() as CFStringRef, value.as_ptr())
    }

    fn set_ax_size(element: AXUIElementRef, attribute: &str, size: CGSize) -> Result<(), String> {
        let attribute = cf_string(attribute)?;
        let value =
            unsafe { AXValueCreate(K_AXVALUE_CGSIZE_TYPE, &size as *const _ as *const c_void) };
        let value = CfOwned::new(value as CFTypeRef)
            .ok_or_else(|| "AXValueCreate(size) returned null.".to_string())?;
        set_ax_value(element, attribute.as_ptr() as CFStringRef, value.as_ptr())
    }

    fn set_ax_value(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> Result<(), String> {
        let err = unsafe { AXUIElementSetAttributeValue(element, attribute, value) };
        if err == AX_ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("AX set attribute failed with error {err}."))
        }
    }

    fn close_window(
        window: AXUIElementRef,
        summary: &MacControlWindowSummary,
        snapshot: &MacControlSnapshot,
    ) -> Result<String, String> {
        if perform_ax_action(window, "AXClose").is_ok() {
            return Ok("AXClose".to_string());
        }
        if let Ok(method) = press_window_close_button(window, summary) {
            return Ok(method);
        }
        let app = summary
            .app_pid
            .and_then(app_summary_for_pid)
            .or_else(|| snapshot.frontmost_app.clone());
        let Some(app) = app.as_ref() else {
            return Err(
                "AXClose and close-button fallback failed; no app target was available for Apple Events fallback."
                    .to_string(),
            );
        };
        let script = if let Some(bundle_id) = app.bundle_id.as_deref() {
            format!(
                "tell application id {} to close front window",
                apple_script_string(bundle_id)
            )
        } else if let Some(name) = app.name.as_deref() {
            format!(
                "tell application {} to close front window",
                apple_script_string(name)
            )
        } else {
            return Err("AXClose failed and no Apple Events app target was available.".to_string());
        };
        if !summary.focused {
            let _ = perform_ax_action(window, "AXRaise");
        }
        run_osascript(&script)?;
        if app.bundle_id.is_some() {
            Ok("AppleEvents.closeByBundleId".to_string())
        } else {
            Ok("AppleEvents.closeByName".to_string())
        }
    }

    fn press_window_close_button(
        window: AXUIElementRef,
        summary: &MacControlWindowSummary,
    ) -> Result<String, String> {
        if let Some(button) = copy_attribute(window, "AXCloseButton") {
            perform_ax_action(button.as_ptr() as AXUIElementRef, "AXPress")?;
            return Ok("AXCloseButton".to_string());
        }
        let button = find_likely_close_button(window, summary)?;
        perform_ax_action(button.as_ptr() as AXUIElementRef, "AXPress")?;
        Ok("AXCloseButtonCandidate".to_string())
    }

    fn find_likely_close_button(
        window: AXUIElementRef,
        summary: &MacControlWindowSummary,
    ) -> Result<CfOwned, String> {
        let Some(window_bounds) = summary.bounds_points else {
            return Err("Window has no bounds for close-button fallback.".to_string());
        };
        let mut best = None;
        find_likely_close_button_inner(window, window_bounds, 0, &mut best);
        best.map(|(_, button)| button)
            .ok_or_else(|| "No close button candidate found for AXPress fallback.".to_string())
    }

    fn find_likely_close_button_inner(
        element: AXUIElementRef,
        window_bounds: MacControlBounds,
        depth: usize,
        best: &mut Option<(i64, CfOwned)>,
    ) {
        if depth > 6 {
            return;
        }
        if attribute_string(element, "AXRole").as_deref() == Some("AXButton") {
            if let Some(bounds) = element_bounds(element) {
                let close_x = window_bounds.x + 16.0;
                let close_y = window_bounds.y + 16.0;
                let center = CGPoint {
                    x: bounds.x + bounds.width / 2.0,
                    y: bounds.y + bounds.height / 2.0,
                };
                let dx = center.x - close_x;
                let dy = center.y - close_y;
                if dx.abs() <= 80.0 && dy.abs() <= 40.0 {
                    let score = ((dx * dx) + (dy * dy)).round() as i64;
                    if best
                        .as_ref()
                        .is_none_or(|(best_score, _)| score < *best_score)
                    {
                        if let Some(retained) =
                            CfOwned::new(unsafe { CFRetain(element as CFTypeRef) })
                        {
                            *best = Some((score, retained));
                        }
                    }
                }
            }
        }
        let children = copy_attribute(element, "AXChildren")
            .or_else(|| copy_attribute(element, "AXVisibleChildren"));
        let Some(children) = children else {
            return;
        };
        for child_ref in cf_array_values(children.as_ptr()) {
            find_likely_close_button_inner(
                child_ref as AXUIElementRef,
                window_bounds,
                depth + 1,
                best,
            );
        }
    }

    fn point_for_element(
        element: &MacControlElementSummary,
        label: &str,
    ) -> Result<CGPoint, String> {
        let bounds = element
            .bounds_points
            .ok_or_else(|| format!("{label} has no bounds for CGEvent fallback."))?;
        center_point(bounds, label)
    }

    fn center_point(bounds: MacControlBounds, label: &str) -> Result<CGPoint, String> {
        if !bounds.x.is_finite()
            || !bounds.y.is_finite()
            || !bounds.width.is_finite()
            || !bounds.height.is_finite()
            || bounds.width < 0.0
            || bounds.height < 0.0
        {
            return Err(format!("{label} has invalid bounds."));
        }
        screen_point(
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
            label,
        )
    }

    fn screen_point(x: f64, y: f64, label: &str) -> Result<CGPoint, String> {
        if !x.is_finite() || !y.is_finite() {
            return Err(format!("{label} coordinates must be finite."));
        }
        Ok(CGPoint { x, y })
    }

    fn post_mouse_click(point: CGPoint, button: MouseButton) -> Result<(), String> {
        post_mouse_click_with_state(point, button, 1)
    }

    fn post_mouse_click_with_state(
        point: CGPoint,
        button: MouseButton,
        click_state: i64,
    ) -> Result<(), String> {
        let (down_type, up_type, cg_button) = match button {
            MouseButton::Left => (
                K_CG_EVENT_LEFT_MOUSE_DOWN,
                K_CG_EVENT_LEFT_MOUSE_UP,
                K_CG_MOUSE_BUTTON_LEFT,
            ),
            MouseButton::Right => (
                K_CG_EVENT_RIGHT_MOUSE_DOWN,
                K_CG_EVENT_RIGHT_MOUSE_UP,
                K_CG_MOUSE_BUTTON_RIGHT,
            ),
        };
        let down = unsafe { CGEventCreateMouseEvent(ptr::null(), down_type, point, cg_button) };
        let up = unsafe { CGEventCreateMouseEvent(ptr::null(), up_type, point, cg_button) };
        let down = CfOwned::new(down as CFTypeRef)
            .ok_or_else(|| "CGEventCreateMouseEvent(down) returned null.".to_string())?;
        let up = CfOwned::new(up as CFTypeRef)
            .ok_or_else(|| "CGEventCreateMouseEvent(up) returned null.".to_string())?;
        unsafe {
            CGEventSetIntegerValueField(down.as_ptr(), K_CG_MOUSE_EVENT_CLICK_STATE, click_state);
            CGEventSetIntegerValueField(up.as_ptr(), K_CG_MOUSE_EVENT_CLICK_STATE, click_state);
            CGEventPost(K_CG_HID_EVENT_TAP, down.as_ptr());
            CGEventPost(K_CG_HID_EVENT_TAP, up.as_ptr());
        }
        Ok(())
    }

    fn post_double_click(point: CGPoint) -> Result<(), String> {
        post_mouse_click_with_state(point, MouseButton::Left, 1)?;
        thread::sleep(Duration::from_millis(60));
        post_mouse_click_with_state(point, MouseButton::Left, 2)
    }

    fn post_mouse_move(to: CGPoint, profile: MotionProfile) -> Result<(), String> {
        let from = current_mouse_position()?;
        for point in motion_points(from, to, profile) {
            let moved = unsafe {
                CGEventCreateMouseEvent(
                    ptr::null(),
                    K_CG_EVENT_MOUSE_MOVED,
                    point,
                    K_CG_MOUSE_BUTTON_LEFT,
                )
            };
            let moved = CfOwned::new(moved as CFTypeRef)
                .ok_or_else(|| "CGEventCreateMouseEvent(mouse moved) returned null.".to_string())?;
            unsafe { CGEventPost(K_CG_HID_EVENT_TAP, moved.as_ptr()) };
            sleep_motion_step(profile);
        }
        Ok(())
    }

    fn post_mouse_drag(
        from: CGPoint,
        to: CGPoint,
        profile: MotionProfile,
        modifiers: &[HotkeyModifier],
    ) -> Result<(), String> {
        let modifier_flags = modifier_flags(modifiers);
        post_modifiers_down(modifiers)?;
        let drag_result = post_mouse_drag_events(from, to, profile, modifier_flags);
        let release_result = post_modifiers(modifiers, false);
        drag_result.and(release_result)
    }

    fn resolve_drag_source(
        request: &MacControlActRequest,
    ) -> Result<(CGPoint, Option<MacControlElementSummary>), String> {
        if !target_query_is_empty(&request.target) {
            if request.from_x.is_some() || request.from_y.is_some() {
                return Err(
                    "act.drag accepts either source target or fromX/fromY, not both.".to_string(),
                );
            }
            let (_element, summary, _) = resolve_element(
                &request.target,
                request.max_elements,
                request.max_depth,
                "act.drag source",
            )?;
            let point = point_for_element(&summary, "act.drag source target")?;
            return Ok((point, Some(summary)));
        }
        let (Some(x), Some(y)) = (request.from_x, request.from_y) else {
            return Err("act.drag requires a source target or fromX/fromY.".to_string());
        };
        Ok((screen_point(x, y, "act.drag source")?, None))
    }

    fn resolve_drag_destination(
        request: &MacControlActRequest,
    ) -> Result<(CGPoint, Option<MacControlElementSummary>), String> {
        if !target_query_is_empty(&request.to_target) {
            if request.x.is_some()
                || request.y.is_some()
                || request.to_x.is_some()
                || request.to_y.is_some()
            {
                return Err(
                    "act.drag accepts only one destination: x/y, toX/toY, or toTarget.".to_string(),
                );
            }
            return resolve_motion_target(request, &request.to_target, "act.drag destination");
        }
        if request.to_x.is_some() || request.to_y.is_some() {
            let (Some(x), Some(y)) = (request.to_x, request.to_y) else {
                return Err("act.drag destination toX/toY requires both toX and toY.".to_string());
            };
            if request.x.is_some() || request.y.is_some() {
                return Err(
                    "act.drag accepts only one destination: x/y, toX/toY, or toTarget.".to_string(),
                );
            }
            return Ok((screen_point(x, y, "act.drag destination")?, None));
        }
        let (Some(x), Some(y)) = (request.x, request.y) else {
            return Err("act.drag requires destination x/y, toX/toY, or toTarget.".to_string());
        };
        Ok((screen_point(x, y, "act.drag destination")?, None))
    }

    fn resolve_swipe_source(
        request: &MacControlActRequest,
    ) -> Result<(CGPoint, Option<MacControlElementSummary>), String> {
        if !target_query_is_empty(&request.target) {
            if request.x.is_some()
                || request.y.is_some()
                || request.from_x.is_some()
                || request.from_y.is_some()
            {
                return Err(
                    "act.swipe accepts only one source: target, x/y, or fromX/fromY.".to_string(),
                );
            }
            return resolve_motion_target(request, &request.target, "act.swipe source");
        }
        if request.from_x.is_some() || request.from_y.is_some() {
            let (Some(x), Some(y)) = (request.from_x, request.from_y) else {
                return Err(
                    "act.swipe start fromX/fromY requires both fromX and fromY.".to_string()
                );
            };
            if request.x.is_some() || request.y.is_some() {
                return Err(
                    "act.swipe accepts only one source: target, x/y, or fromX/fromY.".to_string(),
                );
            }
            return Ok((screen_point(x, y, "act.swipe source")?, None));
        }
        let (Some(x), Some(y)) = (request.x, request.y) else {
            return Err("act.swipe requires start x/y, fromX/fromY, or a target.".to_string());
        };
        Ok((screen_point(x, y, "act.swipe source")?, None))
    }

    fn resolve_swipe_destination(
        request: &MacControlActRequest,
        from: CGPoint,
    ) -> Result<(CGPoint, Option<MacControlElementSummary>), String> {
        if !target_query_is_empty(&request.to_target) {
            if request.delta_x.unwrap_or(0.0) != 0.0
                || request.delta_y.unwrap_or(0.0) != 0.0
                || request.to_x.is_some()
                || request.to_y.is_some()
            {
                return Err(
                    "act.swipe accepts only one destination: deltaX/deltaY, toX/toY, or toTarget."
                        .to_string(),
                );
            }
            return resolve_motion_target(request, &request.to_target, "act.swipe destination");
        }
        if request.to_x.is_some() || request.to_y.is_some() {
            let (Some(x), Some(y)) = (request.to_x, request.to_y) else {
                return Err("act.swipe destination toX/toY requires both toX and toY.".to_string());
            };
            if request.delta_x.unwrap_or(0.0) != 0.0 || request.delta_y.unwrap_or(0.0) != 0.0 {
                return Err(
                    "act.swipe accepts only one destination: deltaX/deltaY, toX/toY, or toTarget."
                        .to_string(),
                );
            }
            return Ok((screen_point(x, y, "act.swipe destination")?, None));
        }
        let delta_x = request.delta_x.unwrap_or(0.0);
        let delta_y = request.delta_y.unwrap_or(0.0);
        if delta_x == 0.0 && delta_y == 0.0 {
            return Err("act.swipe requires deltaX/deltaY, toX/toY, or toTarget.".to_string());
        }
        Ok((
            CGPoint {
                x: from.x + delta_x,
                y: from.y + delta_y,
            },
            None,
        ))
    }

    fn resolve_motion_target(
        request: &MacControlActRequest,
        target_query: &MacControlTargetQuery,
        context: &str,
    ) -> Result<(CGPoint, Option<MacControlElementSummary>), String> {
        let (_element, summary, _) = resolve_element(
            target_query,
            request.max_elements,
            request.max_depth,
            context,
        )?;
        let point = point_for_element(&summary, context)?;
        Ok((point, Some(summary)))
    }

    fn post_mouse_drag_events(
        from: CGPoint,
        to: CGPoint,
        profile: MotionProfile,
        modifier_flags: u64,
    ) -> Result<(), String> {
        let down = unsafe {
            CGEventCreateMouseEvent(
                ptr::null(),
                K_CG_EVENT_LEFT_MOUSE_DOWN,
                from,
                K_CG_MOUSE_BUTTON_LEFT,
            )
        };
        let down = CfOwned::new(down as CFTypeRef)
            .ok_or_else(|| "CGEventCreateMouseEvent(drag down) returned null.".to_string())?;
        unsafe {
            CGEventSetFlags(down.as_ptr(), modifier_flags);
            CGEventPost(K_CG_HID_EVENT_TAP, down.as_ptr());
        }

        for point in motion_points(from, to, profile) {
            let dragged = unsafe {
                CGEventCreateMouseEvent(
                    ptr::null(),
                    K_CG_EVENT_LEFT_MOUSE_DRAGGED,
                    point,
                    K_CG_MOUSE_BUTTON_LEFT,
                )
            };
            let dragged = CfOwned::new(dragged as CFTypeRef).ok_or_else(|| {
                "CGEventCreateMouseEvent(left dragged) returned null.".to_string()
            })?;
            unsafe {
                CGEventSetFlags(dragged.as_ptr(), modifier_flags);
                CGEventPost(K_CG_HID_EVENT_TAP, dragged.as_ptr());
            };
            sleep_motion_step(profile);
        }

        let up = unsafe {
            CGEventCreateMouseEvent(
                ptr::null(),
                K_CG_EVENT_LEFT_MOUSE_UP,
                to,
                K_CG_MOUSE_BUTTON_LEFT,
            )
        };
        let up = CfOwned::new(up as CFTypeRef)
            .ok_or_else(|| "CGEventCreateMouseEvent(drag up) returned null.".to_string())?;
        unsafe {
            CGEventSetFlags(up.as_ptr(), modifier_flags);
            CGEventPost(K_CG_HID_EVENT_TAP, up.as_ptr());
        }
        Ok(())
    }

    fn current_mouse_position() -> Result<CGPoint, String> {
        let event = unsafe { CGEventCreate(ptr::null()) };
        let event = CfOwned::new(event as CFTypeRef)
            .ok_or_else(|| "CGEventCreate(current mouse location) returned null.".to_string())?;
        Ok(unsafe { CGEventGetLocation(event.as_ptr() as CGEventRef) })
    }

    fn motion_points(from: CGPoint, to: CGPoint, profile: MotionProfile) -> Vec<CGPoint> {
        match profile.kind {
            MacControlMotionProfile::Linear => linear_motion_points(from, to, profile.steps),
            MacControlMotionProfile::Human => human_motion_points(from, to, profile.steps),
        }
    }

    fn linear_motion_points(from: CGPoint, to: CGPoint, steps: usize) -> Vec<CGPoint> {
        let steps = steps.max(1);
        (1..=steps)
            .map(|idx| {
                let ratio = idx as f64 / steps as f64;
                mix_point(from, to, ratio)
            })
            .collect()
    }

    fn human_motion_points(from: CGPoint, to: CGPoint, steps: usize) -> Vec<CGPoint> {
        let steps = steps.max(1);
        let distance = point_distance(from, to);
        if steps <= 2 || distance <= f64::EPSILON {
            return linear_motion_points(from, to, steps);
        }

        let dx = to.x - from.x;
        let dy = to.y - from.y;
        let unit_x = dx / distance;
        let unit_y = dy / distance;
        let normal_x = -unit_y;
        let normal_y = unit_x;
        let wobble_amplitude = (distance * 0.015).clamp(0.5, 6.0);
        let overshoot = if distance >= 80.0 && steps >= 8 {
            (distance * 0.025).clamp(2.0, 10.0)
        } else {
            0.0
        };
        let primary_to = CGPoint {
            x: to.x + unit_x * overshoot,
            y: to.y + unit_y * overshoot,
        };
        let primary_steps = if overshoot > 0.0 {
            ((steps * 4) / 5).clamp(1, steps - 1)
        } else {
            steps
        };
        let mut points = Vec::with_capacity(steps);
        for idx in 1..=primary_steps {
            let t = idx as f64 / primary_steps as f64;
            let eased = ease_in_out(t);
            let envelope = 4.0 * t * (1.0 - t);
            let wobble = deterministic_wobble(idx) * wobble_amplitude * envelope;
            let base = mix_point(from, primary_to, eased);
            points.push(CGPoint {
                x: base.x + normal_x * wobble,
                y: base.y + normal_y * wobble,
            });
        }

        let correction_steps = steps - primary_steps;
        for idx in 1..=correction_steps {
            let t = idx as f64 / correction_steps as f64;
            points.push(mix_point(primary_to, to, ease_out(t)));
        }
        if let Some(last) = points.last_mut() {
            *last = to;
        }
        points
    }

    fn mix_point(from: CGPoint, to: CGPoint, ratio: f64) -> CGPoint {
        CGPoint {
            x: from.x + (to.x - from.x) * ratio,
            y: from.y + (to.y - from.y) * ratio,
        }
    }

    fn point_distance(from: CGPoint, to: CGPoint) -> f64 {
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        (dx * dx + dy * dy).sqrt()
    }

    fn ease_in_out(t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    fn ease_out(t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        1.0 - (1.0 - t) * (1.0 - t)
    }

    fn deterministic_wobble(idx: usize) -> f64 {
        (((idx * 37 + 17) % 101) as f64 / 50.0) - 1.0
    }

    fn motion_profile(
        request: &MacControlActRequest,
        default_steps: usize,
        default_duration_ms: u64,
    ) -> MotionProfile {
        let steps = request.steps.unwrap_or(default_steps).max(1);
        let duration_ms = request.duration_ms.unwrap_or(default_duration_ms);
        let kind = request
            .motion_profile
            .unwrap_or(MacControlMotionProfile::Linear);
        MotionProfile {
            steps,
            duration_ms,
            kind,
        }
    }

    fn sleep_motion_step(profile: MotionProfile) {
        if profile.duration_ms == 0 {
            return;
        }
        let delay_ms = profile.duration_ms / profile.steps.max(1) as u64;
        if delay_ms > 0 {
            thread::sleep(Duration::from_millis(delay_ms));
        }
    }

    fn validate_press_keys(keys: &[String], modifiers: &[String]) -> Result<(), String> {
        if keys.is_empty() {
            return Err("act.press requires key or keys.".to_string());
        }
        parse_modifier_keys(modifiers, "act.press modifiers")?;
        for key in keys {
            let key_name = key.to_ascii_lowercase();
            key_code_for_press(&key_name)
                .ok_or_else(|| format!("Unsupported press key '{key}'."))?;
        }
        Ok(())
    }

    fn validate_hotkey_keys(keys: &[String]) -> Result<(), String> {
        let mut key_code = None;
        for key in keys {
            match key.to_ascii_lowercase().as_str() {
                "cmd" | "command" | "meta" | "shift" | "ctrl" | "control" | "alt" | "option" => {}
                other => {
                    key_code = Some(
                        key_code_for(other)
                            .ok_or_else(|| format!("Unsupported hotkey key '{other}'."))?,
                    );
                }
            }
        }
        key_code.ok_or_else(|| "Hotkey requires one non-modifier key.".to_string())?;
        Ok(())
    }

    fn post_press_sequence(
        keys: &[String],
        modifiers: &[String],
        repeat: usize,
        hold_ms: u64,
        interval_ms: u64,
    ) -> Result<(), String> {
        if keys.is_empty() {
            return Err("act.press requires key or keys.".to_string());
        }
        let modifiers = parse_modifier_keys(modifiers, "act.press modifiers")?;
        let key_codes = keys
            .iter()
            .map(|key| {
                let key_name = key.to_ascii_lowercase();
                key_code_for_press(&key_name)
                    .ok_or_else(|| format!("Unsupported press key '{key}'."))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let repeat = repeat.max(1);
        let flags = modifier_flags(&modifiers);
        post_modifiers_down(&modifiers)?;
        let sequence_result = (|| {
            for repeat_idx in 0..repeat {
                for (key_idx, key_code) in key_codes.iter().enumerate() {
                    post_key_press(*key_code, flags, hold_ms)?;
                    if interval_ms > 0 && (repeat_idx + 1 < repeat || key_idx + 1 < key_codes.len())
                    {
                        thread::sleep(Duration::from_millis(interval_ms));
                    }
                }
            }
            Ok(())
        })();
        let release_result = post_modifiers(&modifiers, false);
        sequence_result.and(release_result)
    }

    fn post_key_press(key_code: u16, flags: u64, hold_ms: u64) -> Result<(), String> {
        let source_owner = keyboard_event_source()?;
        let source = source_owner.as_ptr() as CGEventSourceRef;
        post_keyboard_event(source, key_code, true, flags)?;
        if hold_ms > 0 {
            thread::sleep(Duration::from_millis(hold_ms));
        }
        post_keyboard_event(source, key_code, false, flags)
    }

    fn post_modifiers_down(modifiers: &[HotkeyModifier]) -> Result<(), String> {
        match post_modifiers(modifiers, true) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = post_modifiers(modifiers, false);
                Err(error)
            }
        }
    }

    fn post_modifiers(modifiers: &[HotkeyModifier], down: bool) -> Result<(), String> {
        if modifiers.is_empty() {
            return Ok(());
        }
        let source_owner = keyboard_event_source()?;
        let source = source_owner.as_ptr() as CGEventSourceRef;
        if down {
            let mut active_flags = 0_u64;
            for modifier in modifiers {
                active_flags |= modifier.flag;
                post_keyboard_event(source, modifier.key_code, true, active_flags)?;
                thread::sleep(Duration::from_millis(8));
            }
        } else {
            let mut active_flags = modifier_flags(modifiers);
            for modifier in modifiers.iter().rev() {
                active_flags &= !modifier.flag;
                post_keyboard_event(source, modifier.key_code, false, active_flags)?;
                thread::sleep(Duration::from_millis(8));
            }
        }
        Ok(())
    }

    fn parse_modifier_keys(keys: &[String], context: &str) -> Result<Vec<HotkeyModifier>, String> {
        keys.iter()
            .map(|key| {
                modifier_for_key_name(&key.to_ascii_lowercase())
                    .ok_or_else(|| format!("Unsupported {context} key '{key}'."))
            })
            .collect()
    }

    fn modifier_flags(modifiers: &[HotkeyModifier]) -> u64 {
        modifiers
            .iter()
            .fold(0_u64, |flags, modifier| flags | modifier.flag)
    }

    fn modifier_for_key_name(key: &str) -> Option<HotkeyModifier> {
        Some(match key {
            "cmd" | "command" | "meta" => HotkeyModifier {
                key_code: K_VK_COMMAND,
                flag: K_CG_EVENT_FLAG_MASK_COMMAND,
            },
            "shift" => HotkeyModifier {
                key_code: K_VK_SHIFT,
                flag: K_CG_EVENT_FLAG_MASK_SHIFT,
            },
            "ctrl" | "control" => HotkeyModifier {
                key_code: K_VK_CONTROL,
                flag: K_CG_EVENT_FLAG_MASK_CONTROL,
            },
            "alt" | "option" => HotkeyModifier {
                key_code: K_VK_OPTION,
                flag: K_CG_EVENT_FLAG_MASK_ALTERNATE,
            },
            _ => return None,
        })
    }

    fn key_code_for_press(key: &str) -> Option<u16> {
        key_code_for(key).or_else(|| modifier_for_key_name(key).map(|modifier| modifier.key_code))
    }

    fn post_hotkey(keys: &[String]) -> Result<(), String> {
        let mut modifiers = Vec::new();
        let mut key_code = None;
        for key in keys {
            match key.to_ascii_lowercase().as_str() {
                "cmd" | "command" | "meta" => modifiers.push(HotkeyModifier {
                    key_code: K_VK_COMMAND,
                    flag: K_CG_EVENT_FLAG_MASK_COMMAND,
                }),
                "shift" => modifiers.push(HotkeyModifier {
                    key_code: K_VK_SHIFT,
                    flag: K_CG_EVENT_FLAG_MASK_SHIFT,
                }),
                "ctrl" | "control" => modifiers.push(HotkeyModifier {
                    key_code: K_VK_CONTROL,
                    flag: K_CG_EVENT_FLAG_MASK_CONTROL,
                }),
                "alt" | "option" => modifiers.push(HotkeyModifier {
                    key_code: K_VK_OPTION,
                    flag: K_CG_EVENT_FLAG_MASK_ALTERNATE,
                }),
                other => {
                    key_code = Some(
                        key_code_for(other)
                            .ok_or_else(|| format!("Unsupported hotkey key '{other}'."))?,
                    )
                }
            }
        }
        let key_code =
            key_code.ok_or_else(|| "Hotkey requires one non-modifier key.".to_string())?;
        let flags = modifiers
            .iter()
            .fold(0_u64, |flags, item| flags | item.flag);
        if modifiers.is_empty() {
            post_key(key_code, flags)
        } else {
            post_key_chord(key_code, &modifiers, flags)
        }
    }

    fn post_system_events_hotkey(keys: &[String]) -> Result<(), String> {
        let mut modifiers = Vec::new();
        let mut key_code = None;
        for key in keys {
            match key.to_ascii_lowercase().as_str() {
                "cmd" | "command" | "meta" => modifiers.push("command down"),
                "shift" => modifiers.push("shift down"),
                "ctrl" | "control" => modifiers.push("control down"),
                "alt" | "option" => modifiers.push("option down"),
                other => {
                    key_code = Some(
                        key_code_for(other)
                            .ok_or_else(|| format!("Unsupported hotkey key '{other}'."))?,
                    )
                }
            }
        }
        let key_code =
            key_code.ok_or_else(|| "Hotkey requires one non-modifier key.".to_string())?;
        let keystroke = if modifiers.is_empty() {
            format!("key code {key_code}")
        } else {
            format!("key code {key_code} using {{{}}}", modifiers.join(", "))
        };
        let script = format!("tell application \"System Events\" to {keystroke}");
        let output = Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|error| format!("Failed to run System Events hotkey: {error}"))?;
        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(if stderr.is_empty() {
                "System Events hotkey failed with no stderr.".to_string()
            } else {
                stderr
            })
        }
    }

    fn post_key(key_code: u16, flags: u64) -> Result<(), String> {
        let source = keyboard_event_source()?;
        post_keyboard_event(source.as_ptr() as CGEventSourceRef, key_code, true, flags)?;
        thread::sleep(Duration::from_millis(20));
        post_keyboard_event(source.as_ptr() as CGEventSourceRef, key_code, false, flags)?;
        Ok(())
    }

    fn post_key_chord(
        key_code: u16,
        modifiers: &[HotkeyModifier],
        flags: u64,
    ) -> Result<(), String> {
        let source_owner = keyboard_event_source()?;
        let source = source_owner.as_ptr() as CGEventSourceRef;
        let mut active_flags = 0_u64;
        for modifier in modifiers {
            active_flags |= modifier.flag;
            post_keyboard_event(source, modifier.key_code, true, active_flags)?;
            thread::sleep(Duration::from_millis(12));
        }

        post_keyboard_event(source, key_code, true, flags)?;
        thread::sleep(Duration::from_millis(35));
        post_keyboard_event(source, key_code, false, flags)?;
        thread::sleep(Duration::from_millis(12));

        for modifier in modifiers.iter().rev() {
            active_flags &= !modifier.flag;
            post_keyboard_event(source, modifier.key_code, false, active_flags)?;
            thread::sleep(Duration::from_millis(12));
        }
        Ok(())
    }

    fn post_keyboard_event(
        source: CGEventSourceRef,
        key_code: u16,
        key_down: bool,
        flags: u64,
    ) -> Result<(), String> {
        let event = unsafe { CGEventCreateKeyboardEvent(source, key_code, key_down) };
        let event = CfOwned::new(event as CFTypeRef)
            .ok_or_else(|| "CGEventCreateKeyboardEvent returned null.".to_string())?;
        unsafe {
            CGEventSetFlags(event.as_ptr(), flags);
            CGEventPost(K_CG_HID_EVENT_TAP, event.as_ptr());
        }
        Ok(())
    }

    fn post_unicode_text(text: &str, profile: TypingMotionProfile) -> Result<(), String> {
        let source_owner = keyboard_event_source()?;
        let source = source_owner.as_ptr() as CGEventSourceRef;
        for (idx, ch) in text.chars().enumerate() {
            let mut buf = [0_u16; 2];
            let utf16 = ch.encode_utf16(&mut buf).to_vec();
            post_unicode_key_event(source, &utf16, true)?;
            post_unicode_key_event(source, &utf16, false)?;
            sleep_typing_step(profile, idx);
        }
        Ok(())
    }

    fn post_unicode_key_event(
        source: CGEventSourceRef,
        utf16: &[u16],
        key_down: bool,
    ) -> Result<(), String> {
        let event = unsafe { CGEventCreateKeyboardEvent(source, 0, key_down) };
        let event = CfOwned::new(event as CFTypeRef)
            .ok_or_else(|| "CGEventCreateKeyboardEvent(unicode) returned null.".to_string())?;
        unsafe {
            CGEventKeyboardSetUnicodeString(
                event.as_ptr() as CGEventRef,
                utf16.len() as CFIndex,
                utf16.as_ptr(),
            );
            CGEventPost(K_CG_HID_EVENT_TAP, event.as_ptr());
        }
        Ok(())
    }

    fn typing_motion_profile(request: &MacControlActRequest) -> TypingMotionProfile {
        let base_delay_ms = match request.typing_profile {
            Some(MacControlTypingProfile::Instant) => 0,
            Some(MacControlTypingProfile::Steady) => STEADY_TYPING_DELAY_MS,
            Some(MacControlTypingProfile::Human) => HUMAN_TYPING_DELAY_MS,
            None => request.typing_delay_ms.unwrap_or(0),
        };
        TypingMotionProfile {
            base_delay_ms: request.typing_delay_ms.unwrap_or(base_delay_ms),
            human_jitter: request.typing_profile == Some(MacControlTypingProfile::Human),
        }
    }

    fn sleep_typing_step(profile: TypingMotionProfile, idx: usize) {
        let mut delay_ms = profile.base_delay_ms;
        if profile.human_jitter && delay_ms > 0 {
            let jitter = ((idx as i64 * 17 + 11) % 23) - 11;
            if jitter.is_negative() {
                delay_ms = delay_ms.saturating_sub(jitter.unsigned_abs());
            } else {
                delay_ms = delay_ms.saturating_add(jitter as u64);
            }
        }
        if delay_ms > 0 {
            thread::sleep(Duration::from_millis(delay_ms));
        }
    }

    fn keyboard_event_source() -> Result<CfOwned, String> {
        let source = unsafe { CGEventSourceCreate(K_CG_EVENT_SOURCE_STATE_HID_SYSTEM_STATE) };
        CfOwned::new(source as CFTypeRef)
            .ok_or_else(|| "CGEventSourceCreate(HIDSystemState) returned null.".to_string())
    }

    fn post_scroll(delta_x: f64, delta_y: f64) -> Result<(), String> {
        let event = unsafe {
            CGEventCreateScrollWheelEvent(
                ptr::null(),
                K_CG_SCROLL_EVENT_UNIT_LINE,
                2,
                delta_y.round() as i32,
                delta_x.round() as i32,
            )
        };
        let event = CfOwned::new(event as CFTypeRef)
            .ok_or_else(|| "CGEventCreateScrollWheelEvent returned null.".to_string())?;
        unsafe { CGEventPost(K_CG_HID_EVENT_TAP, event.as_ptr()) };
        Ok(())
    }

    fn key_code_for(key: &str) -> Option<u16> {
        Some(match key {
            "a" => 0,
            "s" => 1,
            "d" => 2,
            "f" => 3,
            "h" => 4,
            "g" => 5,
            "z" => 6,
            "x" => 7,
            "c" => 8,
            "v" => 9,
            "b" => 11,
            "q" => 12,
            "w" => 13,
            "e" => 14,
            "r" => 15,
            "y" => 16,
            "t" => 17,
            "1" => 18,
            "2" => 19,
            "3" => 20,
            "4" => 21,
            "6" => 22,
            "5" => 23,
            "=" | "equal" => 24,
            "9" => 25,
            "7" => 26,
            "-" | "minus" => 27,
            "8" => 28,
            "0" => 29,
            "]" | "rightbracket" => 30,
            "o" => 31,
            "u" => 32,
            "[" | "leftbracket" => 33,
            "i" => 34,
            "p" => 35,
            "l" => 37,
            "j" => 38,
            "'" | "quote" => 39,
            "k" => 40,
            ";" | "semicolon" => 41,
            "\\" | "backslash" => 42,
            "," | "comma" => 43,
            "/" | "slash" => 44,
            "n" => 45,
            "m" => 46,
            "." | "period" => 47,
            "tab" => 48,
            "space" => 49,
            "enter" | "return" => 36,
            "escape" | "esc" => 53,
            "delete" | "backspace" => 51,
            "left" | "arrowleft" => 123,
            "right" | "arrowright" => 124,
            "down" | "arrowdown" => 125,
            "up" | "arrowup" => 126,
            _ => return None,
        })
    }

    fn menu_children(element: AXUIElementRef, max_depth: usize) -> Vec<MacControlMenuItemSummary> {
        if max_depth == 0 {
            return Vec::new();
        }
        let Some(children) = copy_attribute(element, "AXChildren")
            .or_else(|| copy_attribute(element, "AXMenuItems"))
        else {
            return Vec::new();
        };
        cf_array_values(children.as_ptr())
            .into_iter()
            .map(|child| menu_item_summary(child as AXUIElementRef, max_depth - 1))
            .collect()
    }

    fn menu_items_for_scope(
        menu_root: AXUIElementRef,
        scope: MacControlMenuScope,
        max_depth: usize,
    ) -> Vec<MacControlMenuItemSummary> {
        let mut items = if scope == MacControlMenuScope::System {
            system_menu_extra_elements(menu_root)
                .into_iter()
                .map(|item| menu_item_summary(item.as_ptr() as AXUIElementRef, max_depth))
                .collect()
        } else {
            menu_children(menu_root, max_depth)
        };
        assign_menu_item_metadata(&mut items, "menu");
        items
    }

    fn assign_menu_item_metadata(items: &mut [MacControlMenuItemSummary], prefix: &str) {
        for (idx, item) in items.iter_mut().enumerate() {
            item.index = Some(idx);
            item.id = Some(format!("{prefix}_{}", idx + 1));
            if !item.children.is_empty() {
                let child_prefix = item.id.clone().unwrap_or_else(|| prefix.to_string());
                assign_menu_item_metadata(&mut item.children, &child_prefix);
            }
        }
    }

    fn menu_item_summary(element: AXUIElementRef, max_depth: usize) -> MacControlMenuItemSummary {
        MacControlMenuItemSummary {
            id: None,
            index: None,
            title: attribute_string(element, "AXTitle"),
            description: attribute_string(element, "AXDescription"),
            value: attribute_string(element, "AXValue"),
            role: attribute_string(element, "AXRole"),
            enabled: attribute_bool(element, "AXEnabled"),
            bounds_points: element_bounds(element),
            actions: action_names(element),
            children: menu_children(element, max_depth),
        }
    }

    fn menu_item_primary_text(item: &MacControlMenuItemSummary) -> Option<String> {
        item.title
            .clone()
            .or_else(|| item.description.clone())
            .or_else(|| item.value.clone())
            .filter(|value| !value.trim().is_empty())
    }

    fn click_menu_index(
        menu_root: AXUIElementRef,
        scope: MacControlMenuScope,
        index: usize,
    ) -> Result<MacControlMenuItemSummary, String> {
        let items = if scope == MacControlMenuScope::System {
            system_menu_extra_elements(menu_root)
        } else {
            direct_menu_children(menu_root)
        };
        let item = items.get(index).ok_or_else(|| {
            format!(
                "Menu index {index} was not found; valid range is 0..{}.",
                items.len().saturating_sub(1)
            )
        })?;
        let item_ref = item.as_ptr() as AXUIElementRef;
        let mut summary = menu_item_summary(item_ref, 2);
        summary.index = Some(index);
        summary.id = Some(format!("menu_{}", index + 1));
        perform_menu_click_action(item_ref)?;
        thread::sleep(Duration::from_millis(180));
        Ok(summary)
    }

    fn click_menu_path(
        menu_root: AXUIElementRef,
        path: &[String],
    ) -> Result<MacControlMenuItemSummary, String> {
        let mut current = menu_root;
        let mut retained_path = Vec::new();
        let mut last = None;
        for part in path {
            let child = find_menu_child(current, part)
                .ok_or_else(|| format!("Menu path component '{part}' was not found."))?;
            let child_ref = child.as_ptr() as AXUIElementRef;
            perform_menu_click_action(child_ref)?;
            thread::sleep(Duration::from_millis(120));
            last = Some(menu_item_summary(child_ref, 2));
            retained_path.push(child);
            current = retained_path
                .last()
                .expect("retained menu path should contain the current element")
                .as_ptr() as AXUIElementRef;
        }
        last.ok_or_else(|| "menu.click requires a non-empty path.".to_string())
    }

    fn system_menu_extra_elements(menu_root: AXUIElementRef) -> Vec<CfOwned> {
        let children = direct_menu_children(menu_root);
        if let Some(group) = children.iter().rev().find(|child| {
            attribute_string(child.as_ptr() as AXUIElementRef, "AXRole").as_deref()
                == Some("AXGroup")
        }) {
            let group_children = direct_menu_children(group.as_ptr() as AXUIElementRef);
            if !group_children.is_empty() {
                return group_children;
            }
        }
        children
            .into_iter()
            .flat_map(|child| {
                let child_ref = child.as_ptr() as AXUIElementRef;
                if attribute_string(child_ref, "AXRole").as_deref() == Some("AXGroup") {
                    let nested = direct_menu_children(child_ref);
                    if !nested.is_empty() {
                        return nested;
                    }
                }
                vec![child]
            })
            .filter(|child| is_likely_menu_extra(child.as_ptr() as AXUIElementRef))
            .collect()
    }

    fn direct_menu_children(element: AXUIElementRef) -> Vec<CfOwned> {
        let Some(children) = copy_attribute(element, "AXChildren")
            .or_else(|| copy_attribute(element, "AXMenuItems"))
        else {
            return Vec::new();
        };
        cf_array_values(children.as_ptr())
            .into_iter()
            .filter_map(|child| CfOwned::new(unsafe { CFRetain(child as CFTypeRef) }))
            .collect()
    }

    fn is_likely_menu_extra(element: AXUIElementRef) -> bool {
        let role = attribute_string(element, "AXRole").unwrap_or_default();
        if matches!(role.as_str(), "AXMenuBarItem" | "AXButton" | "AXMenuItem") {
            return true;
        }
        element_bounds(element).is_some()
            && (!menu_item_match_strings(element).is_empty() || !action_names(element).is_empty())
    }

    fn perform_menu_click_action(element: AXUIElementRef) -> Result<(), String> {
        let mut errors = Vec::new();
        match perform_ax_action(element, "AXShowMenu") {
            Ok(()) => return Ok(()),
            Err(error) => errors.push(format!("AXShowMenu failed: {error}")),
        }
        match perform_ax_action(element, "AXPress") {
            Ok(()) => Ok(()),
            Err(error) => {
                errors.push(format!("AXPress failed: {error}"));
                let bounds = element_bounds(element).ok_or_else(|| errors.join("; "))?;
                let point = center_point(bounds, "menu item fallback")?;
                post_mouse_click(point, MouseButton::Left).map_err(|click_error| {
                    format!(
                        "{}; CGEvent fallback failed: {click_error}",
                        errors.join("; ")
                    )
                })
            }
        }
    }

    fn find_menu_child(element: AXUIElementRef, title: &str) -> Option<CfOwned> {
        let children = copy_attribute(element, "AXChildren")
            .or_else(|| copy_attribute(element, "AXMenuItems"))?;
        let child_refs = cf_array_values(children.as_ptr());
        for child_ref in &child_refs {
            let child = *child_ref as AXUIElementRef;
            if menu_item_matches_exact(child, title) {
                let retained = unsafe { CFRetain(*child_ref as CFTypeRef) };
                return CfOwned::new(retained);
            }
        }
        for child_ref in &child_refs {
            let child = *child_ref as AXUIElementRef;
            if menu_item_matches_contains(child, title) {
                let retained = unsafe { CFRetain(*child_ref as CFTypeRef) };
                return CfOwned::new(retained);
            }
        }
        for child_ref in child_refs {
            let child = child_ref as AXUIElementRef;
            if is_transparent_menu_container(child) {
                if let Some(found) = find_menu_child(child, title) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn menu_item_matches_exact(element: AXUIElementRef, query: &str) -> bool {
        menu_item_match_strings(element)
            .iter()
            .any(|value| value.eq_ignore_ascii_case(query))
    }

    fn menu_item_matches_contains(element: AXUIElementRef, query: &str) -> bool {
        menu_item_match_strings(element)
            .iter()
            .any(|value| contains_ci(Some(value.as_str()), Some(query)))
    }

    fn menu_item_match_strings(element: AXUIElementRef) -> Vec<String> {
        ["AXTitle", "AXDescription", "AXValue"]
            .into_iter()
            .filter_map(|attribute| attribute_string(element, attribute))
            .filter(|value| !value.trim().is_empty())
            .collect()
    }

    fn is_transparent_menu_container(element: AXUIElementRef) -> bool {
        attribute_string(element, "AXTitle")
            .as_deref()
            .is_none_or(str::is_empty)
            && attribute_string(element, "AXRole")
                .as_deref()
                .is_some_and(|role| matches!(role, "AXMenu" | "AXGroup" | "AXMenuItem"))
    }

    fn fallback_running_app_summaries() -> Vec<MacControlRunningApp> {
        let mut system = sysinfo::System::new();
        system.refresh_processes(sysinfo::ProcessesToUpdate::All, false);

        let mut apps = Vec::new();
        for process in system.processes().values() {
            let pid = process.pid().as_u32();
            if pid > i32::MAX as u32 {
                continue;
            }
            if let Some(app) =
                NSRunningApplication::runningApplicationWithProcessIdentifier(pid as i32)
            {
                merge_running_app_summary(&mut apps, running_app_summary(&app));
            }
        }
        apps
    }

    fn merge_running_app_summary(apps: &mut Vec<MacControlRunningApp>, app: MacControlRunningApp) {
        if apps.iter().any(|existing| existing.pid == app.pid) {
            return;
        }
        apps.push(app);
    }

    fn running_app_summary(app: &NSRunningApplication) -> MacControlRunningApp {
        MacControlRunningApp {
            pid: app.processIdentifier(),
            bundle_id: app.bundleIdentifier().as_deref().map(ToString::to_string),
            name: app.localizedName().as_deref().map(ToString::to_string),
            active: app.isActive(),
            hidden: app.isHidden(),
            activation_policy: activation_policy_label(app.activationPolicy()).to_string(),
        }
    }

    fn activation_policy_label(policy: NSApplicationActivationPolicy) -> &'static str {
        if policy == NSApplicationActivationPolicy::Regular {
            "regular"
        } else if policy == NSApplicationActivationPolicy::Accessory {
            "accessory"
        } else if policy == NSApplicationActivationPolicy::Prohibited {
            "prohibited"
        } else {
            "unknown"
        }
    }

    fn contains_ci(actual: Option<&str>, query: Option<&str>) -> bool {
        query
            .filter(|query| !query.is_empty())
            .map_or(true, |query| {
                actual
                    .map(|actual| {
                        actual
                            .to_ascii_lowercase()
                            .contains(&query.to_ascii_lowercase())
                    })
                    .unwrap_or(false)
            })
    }

    fn window_title_matches(actual: Option<&str>, target: &MacControlTargetQuery) -> bool {
        target
            .window_title
            .as_deref()
            .filter(|query| !query.is_empty())
            .map_or(true, |query| {
                string_matches(actual, query, target.window_title_match)
            })
    }

    fn string_matches(actual: Option<&str>, query: &str, strategy: MacControlStringMatch) -> bool {
        actual
            .map(|actual| match strategy {
                MacControlStringMatch::Exact => actual.eq_ignore_ascii_case(query),
                MacControlStringMatch::Contains => contains_ci(Some(actual), Some(query)),
            })
            .unwrap_or(false)
    }

    fn optional_eq_ci(actual: Option<&str>, query: &str) -> bool {
        actual
            .map(|actual| actual.eq_ignore_ascii_case(query))
            .unwrap_or(false)
    }

    fn apple_script_string(value: &str) -> String {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    }

    fn run_osascript(script: &str) -> Result<(), String> {
        let output = Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|e| format!("Failed to run osascript Apple Events fallback: {e}"))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        Err(format!(
            "Apple Events fallback failed{}.",
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        ))
    }

    fn capture_desktop_frame(display_id: Option<u32>) -> Result<MacControlFramePayload, String> {
        let snapshot_id = ha_core::mac_control::new_snapshot_id();
        let frontmost_app = focused_app_summary();
        let captured = capture_display_frame_bytes(display_id)?;
        Ok(build_frame_payload(
            &snapshot_id,
            frontmost_app,
            &captured,
            None,
        ))
    }

    fn handle_ocr(request: MacControlOcrRequest) -> Result<Vec<MacControlOcrRawTextBlock>, String> {
        if !Path::new(&request.screenshot.path).is_file() {
            return Err(format!(
                "mac_control visual OCR screenshot file was not found: {}",
                request.screenshot.path
            ));
        }

        let url = NSURL::fileURLWithPath(&NSString::from_str(&request.screenshot.path));
        let vision_request = VNRecognizeTextRequest::new();
        vision_request.setRecognitionLevel(match request.recognition_level {
            MacControlOcrRecognitionLevel::Fast => VNRequestTextRecognitionLevel::Fast,
            MacControlOcrRecognitionLevel::Accurate => VNRequestTextRecognitionLevel::Accurate,
        });
        vision_request.setUsesLanguageCorrection(true);
        if vision_request.respondsToSelector(sel!(setAutomaticallyDetectsLanguage:)) {
            vision_request.setAutomaticallyDetectsLanguage(true);
        }

        if !request.languages.is_empty() {
            let languages = request
                .languages
                .iter()
                .map(|language| NSString::from_str(language))
                .collect::<Vec<_>>();
            let languages = NSArray::from_retained_slice(&languages);
            vision_request.setRecognitionLanguages(&languages);
        }

        let request_for_array: Retained<VNRequest> =
            vision_request.clone().into_super().into_super();
        let requests = NSArray::from_retained_slice(&[request_for_array]);
        let options = NSDictionary::<VNImageOption, AnyObject>::new();
        let handler = unsafe {
            VNImageRequestHandler::initWithURL_options(
                VNImageRequestHandler::alloc(),
                &url,
                &options,
            )
        };
        handler
            .performRequests_error(&requests)
            .map_err(|error| format!("Vision OCR failed: {}", error.localizedDescription()))?;

        let Some(observations) = vision_request.results() else {
            return Ok(Vec::new());
        };
        let width_px = request.screenshot.width_px as f64;
        let height_px = request.screenshot.height_px as f64;
        let mut blocks = Vec::new();
        for observation in observations.to_vec() {
            let candidates = observation.topCandidates(1);
            let Some(candidate) = candidates.to_vec().into_iter().next() else {
                continue;
            };
            let text = candidate.string().to_string();
            let confidence = candidate.confidence();
            let bbox = unsafe { observation.boundingBox() };
            let image_bounds = MacControlBounds {
                x: bbox.origin.x * width_px,
                y: (1.0 - bbox.origin.y - bbox.size.height) * height_px,
                width: bbox.size.width * width_px,
                height: bbox.size.height * height_px,
            };
            blocks.push(MacControlOcrRawTextBlock {
                text,
                confidence,
                image_bounds,
            });
        }
        Ok(blocks)
    }

    fn capture_desktop_frame_with_id(
        snapshot: &MacControlSnapshot,
        request: &MacControlSnapshotRequest,
    ) -> Result<(MacControlFramePayload, MacControlScreenshotSummary), String> {
        let captured = match request.screenshot_target {
            MacControlScreenshotTarget::Display => capture_display_frame_bytes(request.display_id)?,
            MacControlScreenshotTarget::Window => {
                capture_window_frame_bytes(request.window_id.as_deref(), snapshot)?
            }
        };
        let mut screenshot = ha_core::mac_control::store_screenshot_jpeg(
            &snapshot.snapshot_id,
            &captured.jpeg,
            captured.width_px,
            captured.height_px,
        )?;
        apply_capture_metadata_to_screenshot(&mut screenshot, &captured);
        let frame = build_frame_payload(
            &snapshot.snapshot_id,
            snapshot.frontmost_app.clone(),
            &captured,
            Some(&screenshot),
        );
        Ok((frame, screenshot))
    }

    fn capture_display_frame_bytes(
        display_id: Option<u32>,
    ) -> Result<CapturedDesktopFrame, String> {
        let monitors = Monitor::all().map_err(|e| format!("Failed to list macOS displays: {e}"))?;
        let monitor = if let Some(display_id) = display_id {
            monitors
                .iter()
                .find(|monitor| monitor.id().ok() == Some(display_id))
                .ok_or_else(|| format!("Display id {display_id} was not found."))?
        } else {
            monitors
                .iter()
                .find(|monitor| monitor.is_primary().unwrap_or(false))
                .or_else(|| monitors.first())
                .ok_or_else(|| "No macOS displays detected.".to_string())?
        };
        let display = monitor_display_summary(monitor);
        let rgba_image = monitor.capture_image().map_err(|e| {
            format!("Desktop capture failed; Screen Recording permission may be missing: {e}")
        })?;
        let (jpeg, width_px, height_px) = encode_rgba_as_jpeg(rgba_image, "macOS display frame")?;
        Ok(CapturedDesktopFrame {
            jpeg,
            width_px,
            height_px,
            target: MacControlScreenshotTarget::Display,
            display_id: display.as_ref().map(|display| display.id),
            window_id: None,
            window_title: None,
            bounds_points: display.as_ref().map(|display| display.frame_points),
            scale: display.as_ref().map(|display| display.scale),
        })
    }

    fn capture_window_frame_bytes(
        window_id: Option<&str>,
        snapshot: &MacControlSnapshot,
    ) -> Result<CapturedDesktopFrame, String> {
        let summary = select_snapshot_window_for_capture(window_id, snapshot)?;
        let window = find_xcap_window_for_summary(summary)?;
        let display = display_for_window(summary, snapshot).or_else(|| {
            window
                .current_monitor()
                .ok()
                .and_then(|monitor| monitor_display_summary(&monitor))
        });
        let rgba_image = window.capture_image().map_err(|e| {
            format!(
                "Window capture failed for {}{}; Screen Recording permission may be missing: {e}",
                summary.id,
                summary
                    .title
                    .as_deref()
                    .map(|title| format!(" ({title})"))
                    .unwrap_or_default()
            )
        })?;
        let (jpeg, width_px, height_px) = encode_rgba_as_jpeg(rgba_image, "macOS window frame")?;
        Ok(CapturedDesktopFrame {
            jpeg,
            width_px,
            height_px,
            target: MacControlScreenshotTarget::Window,
            display_id: display.as_ref().map(|display| display.id),
            window_id: Some(summary.id.clone()),
            window_title: summary.title.clone(),
            bounds_points: summary.bounds_points,
            scale: display.as_ref().map(|display| display.scale),
        })
    }

    fn encode_rgba_as_jpeg(
        rgba_image: image::RgbaImage,
        label: &str,
    ) -> Result<(Vec<u8>, u32, u32), String> {
        let width_px = rgba_image.width();
        let height_px = rgba_image.height();
        let rgb_image = image::DynamicImage::ImageRgba8(rgba_image).to_rgb8();
        let mut jpeg = Vec::new();
        let mut encoder = JpegEncoder::new_with_quality(&mut jpeg, 70);
        encoder
            .encode_image(&rgb_image)
            .map_err(|e| format!("Failed to encode {label} as JPEG: {e}"))?;

        Ok((jpeg, width_px, height_px))
    }

    fn build_frame_payload(
        snapshot_id: &str,
        frontmost_app: Option<MacControlAppSummary>,
        captured: &CapturedDesktopFrame,
        screenshot: Option<&MacControlScreenshotSummary>,
    ) -> MacControlFramePayload {
        let jpeg_base64 =
            base64::engine::general_purpose::STANDARD.encode(captured.jpeg.as_slice());
        MacControlFramePayload {
            snapshot_id: snapshot_id.to_string(),
            media_id: screenshot.map(|item| item.media_id.clone()),
            path: screenshot.map(|item| item.path.clone()),
            jpeg_base64,
            width_px: captured.width_px,
            height_px: captured.height_px,
            target: captured.target,
            display_id: captured.display_id,
            window_id: captured.window_id.clone(),
            window_title: captured.window_title.clone(),
            bounds_points: captured.bounds_points,
            scale: captured.scale,
            captured_at: chrono::Utc::now().timestamp_millis(),
            frontmost_app,
            action_id: None,
        }
    }

    fn apply_capture_metadata_to_screenshot(
        screenshot: &mut MacControlScreenshotSummary,
        captured: &CapturedDesktopFrame,
    ) {
        screenshot.target = captured.target;
        screenshot.display_id = captured.display_id;
        screenshot.window_id = captured.window_id.clone();
        screenshot.window_title = captured.window_title.clone();
        screenshot.bounds_points = captured.bounds_points;
        screenshot.scale = captured.scale;
    }

    fn select_snapshot_window_for_capture<'a>(
        window_id: Option<&str>,
        snapshot: &'a MacControlSnapshot,
    ) -> Result<&'a MacControlWindowSummary, String> {
        if let Some(window_id) = window_id {
            return snapshot
                .windows
                .iter()
                .find(|window| window.id == window_id)
                .ok_or_else(|| {
                    format!(
                        "Snapshot window id '{window_id}' was not found; retry with a fresh snapshot."
                    )
                });
        }
        snapshot
            .windows
            .iter()
            .find(|window| window.focused)
            .or_else(|| {
                snapshot
                    .windows
                    .iter()
                    .find(|window| window.bounds_points.is_some())
            })
            .ok_or_else(|| {
                "No frontmost window is available for window screenshot capture.".to_string()
            })
    }

    fn find_xcap_window_for_summary(summary: &MacControlWindowSummary) -> Result<Window, String> {
        let windows =
            Window::all().map_err(|e| format!("Failed to list capturable windows: {e}"))?;
        let mut best: Option<(i64, Window)> = None;
        for window in windows {
            let Some(score) = xcap_window_score(&window, summary) else {
                continue;
            };
            if best
                .as_ref()
                .is_none_or(|(best_score, _)| score > *best_score)
            {
                best = Some((score, window));
            }
        }
        best.map(|(_, window)| window).ok_or_else(|| {
            format!(
                "Unable to match AX window '{}'{} to a capturable macOS window.",
                summary.id,
                summary
                    .title
                    .as_deref()
                    .map(|title| format!(" ({title})"))
                    .unwrap_or_default()
            )
        })
    }

    fn ax_window_number(window: AXUIElementRef) -> Option<u32> {
        let value = copy_attribute(window, "AXWindowNumber")?;
        let raw = cf_value_i64(value.as_ptr())?;
        u32::try_from(raw).ok().filter(|id| *id != 0)
    }

    fn xcap_window_score(window: &Window, summary: &MacControlWindowSummary) -> Option<i64> {
        let mut score = 0_i64;
        if let Some(pid) = summary.app_pid {
            let window_pid = window.pid().ok()?;
            if window_pid != pid as u32 {
                return None;
            }
            score += 1_000;
        }

        let window_title = window.title().unwrap_or_default();
        if let Some(expected_title) = summary.title.as_deref().filter(|title| !title.is_empty()) {
            if window_title.eq_ignore_ascii_case(expected_title) {
                score += 300;
            } else if !window_title.is_empty()
                && (contains_ci(Some(&window_title), Some(expected_title))
                    || contains_ci(Some(expected_title), Some(&window_title)))
            {
                score += 120;
            } else if summary.app_pid.is_none() {
                return None;
            }
        }

        if let Some(expected_bounds) = summary.bounds_points {
            if let Some(actual_bounds) = xcap_window_bounds(window) {
                let distance = bounds_distance(expected_bounds, actual_bounds).round() as i64;
                if distance <= 12 {
                    score += 240;
                } else if distance <= 80 {
                    score += 120_i64.saturating_sub(distance);
                } else if summary.app_pid.is_none()
                    && summary.title.as_deref().is_none_or(str::is_empty)
                {
                    return None;
                }
            }
        }

        (score > 0).then_some(score)
    }

    fn xcap_window_bounds(window: &Window) -> Option<MacControlBounds> {
        Some(MacControlBounds {
            x: f64::from(window.x().ok()?),
            y: f64::from(window.y().ok()?),
            width: f64::from(window.width().ok()?),
            height: f64::from(window.height().ok()?),
        })
    }

    fn bounds_distance(a: MacControlBounds, b: MacControlBounds) -> f64 {
        (a.x - b.x).abs()
            + (a.y - b.y).abs()
            + (a.width - b.width).abs()
            + (a.height - b.height).abs()
    }

    fn display_for_window(
        window: &MacControlWindowSummary,
        snapshot: &MacControlSnapshot,
    ) -> Option<MacControlDisplaySummary> {
        let bounds = window.bounds_points?;
        let center_x = bounds.x + bounds.width / 2.0;
        let center_y = bounds.y + bounds.height / 2.0;
        snapshot
            .displays
            .iter()
            .find(|display| point_in_bounds(center_x, center_y, display.frame_points))
            .cloned()
            .or_else(|| {
                snapshot
                    .displays
                    .iter()
                    .find(|display| bounds_intersect(bounds, display.frame_points))
                    .cloned()
            })
    }

    fn point_in_bounds(x: f64, y: f64, bounds: MacControlBounds) -> bool {
        x >= bounds.x
            && y >= bounds.y
            && x <= bounds.x + bounds.width
            && y <= bounds.y + bounds.height
    }

    fn bounds_intersect(a: MacControlBounds, b: MacControlBounds) -> bool {
        a.x < b.x + b.width && a.x + a.width > b.x && a.y < b.y + b.height && a.y + a.height > b.y
    }

    fn display_summaries() -> Result<Vec<MacControlDisplaySummary>, String> {
        let monitors = Monitor::all().map_err(|e| format!("Failed to list macOS displays: {e}"))?;
        Ok(monitors
            .iter()
            .filter_map(|monitor| monitor_display_summary(monitor))
            .collect())
    }

    fn monitor_display_summary(monitor: &Monitor) -> Option<MacControlDisplaySummary> {
        let scale = monitor.scale_factor().ok().map(f64::from).unwrap_or(1.0);
        Some(MacControlDisplaySummary {
            id: monitor.id().ok()?,
            frame_points: MacControlBounds {
                x: f64::from(monitor.x().ok()?),
                y: f64::from(monitor.y().ok()?),
                width: f64::from(monitor.width().ok()?),
                height: f64::from(monitor.height().ok()?),
            },
            scale,
        })
    }

    fn focused_app_summary() -> Option<MacControlAppSummary> {
        let system = unsafe { AXUIElementCreateSystemWide() };
        let system = CfOwned::new(system as CFTypeRef)?;
        let app = copy_attribute(system.as_ptr() as AXUIElementRef, "AXFocusedApplication")?;
        Some(app_summary(app.as_ptr() as AXUIElementRef))
    }

    fn app_summary_for_pid(pid: i32) -> Option<MacControlAppSummary> {
        let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
        let summary = running_app_summary(&app);
        Some(MacControlAppSummary {
            pid: summary.pid,
            bundle_id: summary.bundle_id,
            name: summary.name,
        })
    }

    fn app_element_for_pid(pid: i32) -> Option<CfOwned> {
        if pid <= 0 {
            return None;
        }
        let app = unsafe { AXUIElementCreateApplication(pid) };
        CfOwned::new(app as CFTypeRef)
    }

    fn focused_overlay_roots(app: AXUIElementRef) -> Vec<FocusedOverlayRoot> {
        let Some(mut current) = copy_attribute(app, "AXFocusedUIElement") else {
            return Vec::new();
        };
        let mut dialog_root = None;
        let mut parent_window = None;
        for _ in 0..8 {
            let current_ref = current.as_ptr() as AXUIElementRef;
            let role = attribute_string(current_ref, "AXRole").unwrap_or_default();
            if role == "AXWindow" {
                let retained = unsafe { CFRetain(current_ref as CFTypeRef) };
                parent_window = CfOwned::new(retained);
                break;
            }
            if role == "AXApplication" {
                break;
            }
            if role_name_is_dialogish(&role) {
                let retained = unsafe { CFRetain(current_ref as CFTypeRef) };
                dialog_root = CfOwned::new(retained);
            }
            let Some(parent) = copy_attribute(current_ref, "AXParent") else {
                break;
            };
            current = parent;
        }
        let Some(element) = dialog_root else {
            return Vec::new();
        };
        let window_id = parent_window
            .as_ref()
            .and_then(|window| window_id_for_app_window(app, window.as_ptr() as AXUIElementRef));
        vec![FocusedOverlayRoot { element, window_id }]
    }

    fn window_id_for_app_window(
        app: AXUIElementRef,
        target_window: AXUIElementRef,
    ) -> Option<String> {
        let windows = copy_attribute(app, "AXWindows")?;
        cf_array_values(windows.as_ptr())
            .into_iter()
            .enumerate()
            .find_map(|(idx, window_ref)| {
                ax_elements_equal(window_ref as AXUIElementRef, target_window)
                    .then(|| format!("win_{}", idx + 1))
            })
    }

    fn ax_elements_equal(left: AXUIElementRef, right: AXUIElementRef) -> bool {
        left == right || unsafe { CFEqual(left as CFTypeRef, right as CFTypeRef) != 0 }
    }

    fn role_name_is_dialogish(role: &str) -> bool {
        let role = role.to_ascii_lowercase();
        role.contains("dialog")
            || role.contains("sheet")
            || role.contains("systemdialog")
            || role.contains("popover")
    }

    fn traverse_focused_overlay_roots(app: AXUIElementRef, state: &mut CaptureState) {
        for root in focused_overlay_roots(app) {
            traverse_element(
                root.element.as_ptr() as AXUIElementRef,
                0,
                root.window_id.as_deref(),
                state,
            );
            if state.truncated {
                break;
            }
        }
    }

    fn app_summary(app: AXUIElementRef) -> MacControlAppSummary {
        let pid = ax_pid(app).unwrap_or_default();
        let running_app = if pid > 0 {
            NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        } else {
            None
        };
        MacControlAppSummary {
            pid,
            bundle_id: running_app
                .as_deref()
                .and_then(|app| app.bundleIdentifier())
                .as_deref()
                .map(ToString::to_string),
            name: running_app
                .as_deref()
                .and_then(|app| app.localizedName())
                .as_deref()
                .map(ToString::to_string)
                .or_else(|| attribute_string(app, "AXTitle")),
        }
    }

    fn window_summary(window: AXUIElementRef, id: &str) -> MacControlWindowSummary {
        MacControlWindowSummary {
            id: id.to_string(),
            app_pid: ax_pid(window),
            role: attribute_string(window, "AXRole"),
            subrole: attribute_string(window, "AXSubrole"),
            title: attribute_string(window, "AXTitle"),
            focused: attribute_bool(window, "AXFocused").unwrap_or(false),
            bounds_points: element_bounds(window),
        }
    }

    fn window_summary_for_app(
        window: AXUIElementRef,
        id: &str,
        app_pid: Option<i32>,
    ) -> MacControlWindowSummary {
        let mut summary = window_summary(window, id);
        if summary.app_pid.is_none() {
            summary.app_pid = app_pid;
        }
        summary
    }

    fn traverse_element(
        element: AXUIElementRef,
        depth: usize,
        window_id: Option<&str>,
        state: &mut CaptureState,
    ) {
        if state.elements.len() >= state.max_elements {
            state.truncated = true;
            return;
        }

        let summary = element_summary(element, window_id, state.next_element_id);
        if should_include_element(&summary) {
            state.next_element_id += 1;
            state.elements.push(summary);
            if state.elements.len() >= state.max_elements {
                state.truncated = true;
                return;
            }
        }

        if depth >= state.max_depth {
            return;
        }

        let children = copy_attribute(element, "AXChildren")
            .or_else(|| copy_attribute(element, "AXVisibleChildren"));
        let Some(children) = children else {
            return;
        };
        for child_ref in cf_array_values(children.as_ptr()) {
            traverse_element(child_ref as AXUIElementRef, depth + 1, window_id, state);
            if state.truncated {
                break;
            }
        }
    }

    fn element_summary(
        element: AXUIElementRef,
        window_id: Option<&str>,
        element_index: usize,
    ) -> MacControlElementSummary {
        let role = attribute_string(element, "AXRole");
        let label = attribute_string(element, "AXTitle")
            .or_else(|| attribute_string(element, "AXDescription"))
            .or_else(|| attribute_string(element, "AXHelp"));
        let value = attribute_string(element, "AXValue")
            .filter(|value| label.as_ref().map(|label| label != value).unwrap_or(true));
        let focused = attribute_bool(element, "AXFocused").unwrap_or(false);
        let actions = action_names(element);
        let id = format!("el_{element_index}");

        MacControlElementSummary {
            id,
            window_id: window_id.map(str::to_string),
            role,
            label,
            value,
            enabled: attribute_bool(element, "AXEnabled"),
            focused,
            bounds_points: element_bounds(element),
            actions,
        }
    }

    fn should_include_element(element: &MacControlElementSummary) -> bool {
        if !element.actions.is_empty() || element.focused {
            return true;
        }
        let role = element
            .role
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let interesting_role = [
            "button", "checkbox", "combobox", "dialog", "link", "menu", "outline", "pop", "radio",
            "row", "search", "sheet", "slider", "tab", "text", "webarea",
        ]
        .iter()
        .any(|needle| role.contains(needle));
        interesting_role || (element.bounds_points.is_some() && element.label.is_some())
    }

    fn copy_attribute(element: AXUIElementRef, attribute: &str) -> Option<CfOwned> {
        let attribute = cf_string(attribute).ok()?;
        let mut value: CFTypeRef = ptr::null();
        let err = unsafe {
            AXUIElementCopyAttributeValue(
                element,
                attribute.as_ptr() as CFStringRef,
                &mut value as *mut CFTypeRef,
            )
        };
        if err == AX_ERROR_SUCCESS {
            CfOwned::new(value)
        } else {
            None
        }
    }

    fn action_names(element: AXUIElementRef) -> Vec<String> {
        let mut names: CFArrayRef = ptr::null();
        let err = unsafe { AXUIElementCopyActionNames(element, &mut names as *mut CFArrayRef) };
        if err != AX_ERROR_SUCCESS {
            return Vec::new();
        }
        let Some(names) = CfOwned::new(names as CFTypeRef) else {
            return Vec::new();
        };
        cf_array_strings(names.as_ptr())
    }

    fn attribute_string(element: AXUIElementRef, attribute: &str) -> Option<String> {
        let value = copy_attribute(element, attribute)?;
        cf_value_string(value.as_ptr())
    }

    fn attribute_bool(element: AXUIElementRef, attribute: &str) -> Option<bool> {
        let value = copy_attribute(element, attribute)?;
        cf_bool(value.as_ptr())
    }

    fn ax_pid(element: AXUIElementRef) -> Option<i32> {
        let mut pid = 0_i32;
        let err = unsafe { AXUIElementGetPid(element, &mut pid as *mut i32) };
        (err == AX_ERROR_SUCCESS).then_some(pid)
    }

    fn element_bounds(element: AXUIElementRef) -> Option<MacControlBounds> {
        if let Some(frame) =
            copy_attribute(element, "AXFrame").and_then(|value| ax_rect(value.as_ptr()))
        {
            return Some(frame);
        }
        let position =
            copy_attribute(element, "AXPosition").and_then(|value| ax_point(value.as_ptr()))?;
        let size = copy_attribute(element, "AXSize").and_then(|value| ax_size(value.as_ptr()))?;
        Some(MacControlBounds {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        })
    }

    fn ax_rect(value: CFTypeRef) -> Option<MacControlBounds> {
        let mut rect = CGRect::default();
        let ok = unsafe {
            AXValueGetType(value as AXValueRef) == K_AXVALUE_CGRECT_TYPE
                && AXValueGetValue(
                    value as AXValueRef,
                    K_AXVALUE_CGRECT_TYPE,
                    &mut rect as *mut CGRect as *mut c_void,
                ) != 0
        };
        ok.then_some(MacControlBounds {
            x: rect.origin.x,
            y: rect.origin.y,
            width: rect.size.width,
            height: rect.size.height,
        })
    }

    fn ax_point(value: CFTypeRef) -> Option<CGPoint> {
        let mut point = CGPoint::default();
        let ok = unsafe {
            AXValueGetType(value as AXValueRef) == K_AXVALUE_CGPOINT_TYPE
                && AXValueGetValue(
                    value as AXValueRef,
                    K_AXVALUE_CGPOINT_TYPE,
                    &mut point as *mut CGPoint as *mut c_void,
                ) != 0
        };
        ok.then_some(point)
    }

    fn ax_size(value: CFTypeRef) -> Option<CGSize> {
        let mut size = CGSize::default();
        let ok = unsafe {
            AXValueGetType(value as AXValueRef) == K_AXVALUE_CGSIZE_TYPE
                && AXValueGetValue(
                    value as AXValueRef,
                    K_AXVALUE_CGSIZE_TYPE,
                    &mut size as *mut CGSize as *mut c_void,
                ) != 0
        };
        ok.then_some(size)
    }

    fn cf_string(value: &str) -> Result<CfOwned, String> {
        let value = CString::new(value).map_err(|e| format!("invalid CFString value: {e}"))?;
        let ptr = unsafe {
            CFStringCreateWithCString(ptr::null(), value.as_ptr(), K_CFSTRING_ENCODING_UTF8)
        };
        CfOwned::new(ptr as CFTypeRef)
            .ok_or_else(|| "CFStringCreateWithCString returned null".to_string())
    }

    fn cf_number(value: i64) -> Result<CfOwned, String> {
        let ptr = unsafe {
            CFNumberCreate(
                ptr::null(),
                K_CFNUMBER_SINT64_TYPE,
                &value as *const i64 as *const c_void,
            )
        };
        CfOwned::new(ptr).ok_or_else(|| "CFNumberCreate returned null.".to_string())
    }

    fn cf_number_array(values: &[i64], label: &str) -> Result<CfOwned, String> {
        let mut numbers = Vec::with_capacity(values.len());
        for value in values {
            numbers.push(cf_number(*value)?);
        }
        let refs = numbers
            .iter()
            .map(CfOwned::as_ptr)
            .collect::<Vec<CFTypeRef>>();
        let count = CFIndex::try_from(refs.len())
            .map_err(|_| format!("Too many {label} values for CFArray."))?;
        let array = unsafe {
            CFArrayCreate(
                ptr::null(),
                refs.as_ptr(),
                count,
                &raw const kCFTypeArrayCallBacks,
            )
        };
        CfOwned::new(array as CFTypeRef)
            .ok_or_else(|| format!("CFArrayCreate returned null for {label} values."))
    }

    fn cf_value_i64(value: CFTypeRef) -> Option<i64> {
        if value.is_null() || unsafe { CFGetTypeID(value) } != unsafe { CFNumberGetTypeID() } {
            return None;
        }
        let mut result = 0_i64;
        let ok = unsafe {
            CFNumberGetValue(
                value,
                K_CFNUMBER_SINT64_TYPE,
                &mut result as *mut i64 as *mut c_void,
            )
        };
        (ok != 0).then_some(result)
    }

    fn cf_value_string(value: CFTypeRef) -> Option<String> {
        if value.is_null() {
            return None;
        }
        let type_id = unsafe { CFGetTypeID(value) };
        if type_id == unsafe { CFStringGetTypeID() } {
            return cf_string_to_rust(value as CFStringRef);
        }
        if type_id == unsafe { CFBooleanGetTypeID() } {
            return Some(cf_bool(value)?.to_string());
        }
        None
    }

    fn cf_string_to_rust(value: CFStringRef) -> Option<String> {
        let len = unsafe { CFStringGetLength(value) };
        let max_len =
            unsafe { CFStringGetMaximumSizeForEncoding(len, K_CFSTRING_ENCODING_UTF8) + 1 };
        if max_len <= 0 {
            return Some(String::new());
        }
        let mut buffer = vec![0 as c_char; max_len as usize];
        let ok = unsafe {
            CFStringGetCString(
                value,
                buffer.as_mut_ptr(),
                max_len,
                K_CFSTRING_ENCODING_UTF8,
            )
        };
        if ok == 0 {
            return None;
        }
        unsafe { CStr::from_ptr(buffer.as_ptr()) }
            .to_str()
            .ok()
            .map(str::to_string)
    }

    fn cf_bool(value: CFTypeRef) -> Option<bool> {
        if value.is_null() || unsafe { CFGetTypeID(value) } != unsafe { CFBooleanGetTypeID() } {
            return None;
        }
        Some(unsafe { CFBooleanGetValue(value) != 0 })
    }

    fn cf_array_values(value: CFTypeRef) -> Vec<CFTypeRef> {
        if value.is_null() || unsafe { CFGetTypeID(value) } != unsafe { CFArrayGetTypeID() } {
            return Vec::new();
        }
        let array = value as CFArrayRef;
        let count = unsafe { CFArrayGetCount(array) };
        (0..count)
            .filter_map(|idx| {
                let item = unsafe { CFArrayGetValueAtIndex(array, idx) };
                (!item.is_null()).then_some(item as CFTypeRef)
            })
            .collect()
    }

    fn cf_array_strings(value: CFTypeRef) -> Vec<String> {
        cf_array_values(value)
            .into_iter()
            .filter_map(|item| cf_value_string(item))
            .collect()
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub fn register() {}
}

pub fn register() {
    imp::register();
}
