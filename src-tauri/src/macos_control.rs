//! Desktop macOS control bridge.
//!
//! Registers the authorized desktop process and exposes Accessibility
//! snapshots, display/window JPEG frames, app launch/focus, window operations,
//! AX-first element actions, dialogs, and menu inspection/clicks.

#[cfg(target_os = "macos")]
mod imp {
    use std::collections::BTreeSet;
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
        MacControlActOp, MacControlActRequest, MacControlActResult, MacControlAppNameMatch,
        MacControlAppSummary, MacControlAppsOp, MacControlAppsRequest, MacControlAppsResult,
        MacControlBounds, MacControlBridge, MacControlClipboardOp, MacControlClipboardRequest,
        MacControlClipboardResult, MacControlDialogOp, MacControlDialogRequest,
        MacControlDialogResult, MacControlDialogSummary, MacControlDisplaySummary,
        MacControlElementSummary, MacControlFramePayload, MacControlInstalledApp,
        MacControlMenuItemSummary, MacControlMenuOp, MacControlMenuRequest, MacControlMenuResult,
        MacControlMenuScope, MacControlRunningApp, MacControlScreenshotSummary,
        MacControlScreenshotTarget, MacControlSnapshot, MacControlSnapshotRequest,
        MacControlStringMatch, MacControlTargetQuery, MacControlWindowSummary, MacControlWindowsOp,
        MacControlWindowsRequest, MacControlWindowsResult, MacControlWindowsScope,
    };
    use image::codecs::jpeg::JpegEncoder;
    use objc2::rc::Retained;
    use objc2_app_kit::{
        NSApplicationActivationOptions, NSApplicationActivationPolicy, NSRunningApplication,
        NSWorkspace,
    };
    use objc2_foundation::{NSBundle, NSString};
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

        async fn capture_frame(&self) -> Result<MacControlFramePayload, String> {
            tokio::task::spawn_blocking(capture_desktop_frame)
                .await
                .map_err(|e| format!("macOS frame worker failed: {e}"))?
        }

        async fn apps(
            &self,
            request: MacControlAppsRequest,
        ) -> Result<MacControlAppsResult, String> {
            tokio::task::spawn_blocking(move || handle_apps(request))
                .await
                .map_err(|e| format!("macOS apps worker failed: {e}"))?
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
    type AXUIElementRef = *const c_void;
    type AXValueRef = *const c_void;
    type CGEventRef = *const c_void;
    type CGEventSourceRef = *const c_void;

    const AX_ERROR_SUCCESS: AXError = 0;
    const K_CFSTRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const K_AXVALUE_CGPOINT_TYPE: i32 = 1;
    const K_AXVALUE_CGSIZE_TYPE: i32 = 2;
    const K_AXVALUE_CGRECT_TYPE: i32 = 3;
    const K_CG_HID_EVENT_TAP: u32 = 0;
    const K_CG_EVENT_LEFT_MOUSE_DOWN: u32 = 1;
    const K_CG_EVENT_LEFT_MOUSE_UP: u32 = 2;
    const K_CG_EVENT_RIGHT_MOUSE_DOWN: u32 = 3;
    const K_CG_EVENT_RIGHT_MOUSE_UP: u32 = 4;
    const K_CG_EVENT_LEFT_MOUSE_DRAGGED: u32 = 6;
    const K_CG_MOUSE_BUTTON_LEFT: u32 = 0;
    const K_CG_MOUSE_BUTTON_RIGHT: u32 = 1;
    const K_CG_MOUSE_EVENT_CLICK_STATE: u32 = 1;
    const K_CG_SCROLL_EVENT_UNIT_LINE: u32 = 1;
    const K_CG_EVENT_FLAG_MASK_SHIFT: u64 = 0x0002_0000;
    const K_CG_EVENT_FLAG_MASK_CONTROL: u64 = 0x0004_0000;
    const K_CG_EVENT_FLAG_MASK_ALTERNATE: u64 = 0x0008_0000;
    const K_CG_EVENT_FLAG_MASK_COMMAND: u64 = 0x0010_0000;

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

    #[derive(Clone, Copy)]
    enum MouseButton {
        Left,
        Right,
    }

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
        fn CGEventCreateScrollWheelEvent(
            source: CGEventSourceRef,
            units: u32,
            wheel_count: u32,
            wheel1: i32,
            ...
        ) -> CGEventRef;
        fn CGEventSetFlags(event: CGEventRef, flags: u64);
        fn CGEventSetIntegerValueField(event: CGEventRef, field: u32, value: i64);
        fn CGEventPost(tap: u32, event: CGEventRef);
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFBooleanTrue: CFTypeRef;
        fn CFRelease(cf: CFTypeRef);
        fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
        fn CFGetTypeID(cf: CFTypeRef) -> CFTypeID;
        fn CFStringGetTypeID() -> CFTypeID;
        fn CFArrayGetTypeID() -> CFTypeID;
        fn CFBooleanGetTypeID() -> CFTypeID;
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
        fn CFBooleanGetValue(boolean: CFTypeRef) -> Boolean;
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

        let mut state = CaptureState {
            max_elements: request.max_elements,
            max_depth: request.max_depth,
            next_element_id: 1,
            elements: Vec::new(),
            truncated: false,
        };

        if let Some(windows) = copy_attribute(app_ref, "AXWindows") {
            for (idx, window_ref) in cf_array_values(windows.as_ptr()).into_iter().enumerate() {
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

    fn handle_windows(
        request: MacControlWindowsRequest,
    ) -> Result<MacControlWindowsResult, String> {
        let request = request.clamped();
        let frontmost_app = focused_app_summary();
        let mut windows = list_windows_for_request(&request)?;
        let mut execution = None;
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
            Some(window_summary(
                window.as_ptr() as AXUIElementRef,
                &summary.id,
            ))
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
        })
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

    fn handle_act(request: MacControlActRequest) -> Result<MacControlActResult, String> {
        let request = request.clamped();
        let mut target = None;
        let execution = match request.op {
            MacControlActOp::Click => {
                if target_query_is_empty(&request.target) {
                    return Err(
                        "act.click requires a target; use act.click_point for raw x/y coordinates."
                            .to_string(),
                    );
                }
                let (element, summary, _) =
                    resolve_element(&request.target, request.max_elements, request.max_depth)?;
                let element_ref = element.as_ptr() as AXUIElementRef;
                target = Some(summary.clone());
                if summary.actions.iter().any(|action| action == "AXPress") {
                    perform_ax_action(element_ref, "AXPress")?;
                    "AXPress".to_string()
                } else {
                    let bounds = summary.bounds_points.ok_or_else(|| {
                        "act.click target has no AXPress action and no bounds for CGEvent fallback."
                            .to_string()
                    })?;
                    post_mouse_click(center_point(bounds, "act.click target")?, MouseButton::Left)?;
                    "CGEventFallback".to_string()
                }
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
            MacControlActOp::DoubleClick => {
                if target_query_is_empty(&request.target) {
                    return Err("act.double_click requires a target.".to_string());
                }
                let (_element, summary, _) =
                    resolve_element(&request.target, request.max_elements, request.max_depth)?;
                let point = point_for_element(&summary, "act.double_click target")?;
                post_double_click(point)?;
                target = Some(summary);
                "CGEventDoubleClick".to_string()
            }
            MacControlActOp::RightClick => {
                if target_query_is_empty(&request.target) {
                    return Err("act.right_click requires a target.".to_string());
                }
                let (_element, summary, _) =
                    resolve_element(&request.target, request.max_elements, request.max_depth)?;
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
                    )?;
                    (element, summary)
                };
                set_ax_string(element.as_ptr() as AXUIElementRef, "AXValue", text)?;
                target = Some(summary);
                "AXSetValue".to_string()
            }
            MacControlActOp::SetValue => {
                let value = request
                    .value
                    .as_deref()
                    .ok_or_else(|| "act.set_value requires value.".to_string())?;
                let (element, summary, _) =
                    resolve_element(&request.target, request.max_elements, request.max_depth)?;
                set_ax_string(element.as_ptr() as AXUIElementRef, "AXValue", value)?;
                target = Some(summary);
                "AXSetValue".to_string()
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
            MacControlActOp::Scroll => {
                post_scroll(
                    request.delta_x.unwrap_or(0.0),
                    request.delta_y.unwrap_or(0.0),
                )?;
                "CGEventScroll".to_string()
            }
            MacControlActOp::Drag => {
                if target_query_is_empty(&request.target) {
                    return Err("act.drag requires a source target.".to_string());
                }
                let (Some(x), Some(y)) = (request.x, request.y) else {
                    return Err("act.drag requires destination x and y.".to_string());
                };
                let (_element, summary, _) =
                    resolve_element(&request.target, request.max_elements, request.max_depth)?;
                let from = point_for_element(&summary, "act.drag source target")?;
                let to = screen_point(x, y, "act.drag destination")?;
                post_mouse_drag(from, to)?;
                target = Some(summary);
                "CGEventDrag".to_string()
            }
        };
        let snapshot = capture_ax_snapshot(MacControlSnapshotRequest {
            include_screenshot: false,
            max_elements: request.max_elements,
            max_depth: request.max_depth,
            ..Default::default()
        })
        .ok();
        Ok(MacControlActResult {
            op: request.op,
            execution,
            target,
            snapshot,
        })
    }

    fn handle_menu(request: MacControlMenuRequest) -> Result<MacControlMenuResult, String> {
        let request = request.clamped();
        let menu_bar = menu_root_for_scope(request.scope)?;
        let menu_bar_ref = menu_bar.as_ptr() as AXUIElementRef;
        let items = menu_children(menu_bar_ref, request.max_depth);
        let clicked = if request.op == MacControlMenuOp::Click {
            Some(click_menu_path(menu_bar_ref, &request.path)?)
        } else {
            None
        };

        Ok(MacControlMenuResult {
            op: request.op,
            scope: request.scope,
            path: request.path,
            items,
            clicked,
        })
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
                let text_len = text.chars().count();
                clipboard
                    .set_text(text)
                    .map_err(|e| format!("Failed to set clipboard text: {e}"))?;
                Ok(MacControlClipboardResult {
                    op: request.op,
                    text: None,
                    text_len,
                    truncated: false,
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
        let mut execution = None;
        if matches!(
            request.op,
            MacControlDialogOp::Accept | MacControlDialogOp::Dismiss
        ) {
            let button = select_dialog_button(&dialogs, &request).ok_or_else(|| {
                format!(
                    "No dialog button matched dialog.{}.",
                    dialog_op_name(request.op)
                )
            })?;
            let element =
                resolve_element_by_summary(&button, request.max_elements, request.max_depth)?;
            press_dialog_button(element.as_ptr() as AXUIElementRef, &button)?;
            acted_button = Some(button);
            execution = Some("AXPressOrCGEvent".to_string());
        }

        Ok(MacControlDialogResult {
            op: request.op,
            dialogs,
            acted_button,
            snapshot: Some(snapshot),
            execution,
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
            || subrole.contains("dialog")
            || subrole.contains("sheet")
            || subrole.contains("systemdialog")
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
        role.contains("dialog") || role.contains("sheet") || role.contains("systemdialog")
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
            MacControlDialogOp::Inspect => &[],
        };
        dialogs
            .iter()
            .flat_map(|dialog| dialog.buttons.iter())
            .filter(|button| button.enabled != Some(false))
            .max_by_key(|button| dialog_button_score(button, patterns))
            .filter(|button| dialog_button_score(button, patterns) > 0)
            .cloned()
    }

    const ACCEPT_DIALOG_BUTTONS: &[&str] = &[
        "ok", "open", "save", "allow", "continue", "done", "yes", "replace", "好", "确定", "打開",
        "打开", "儲存", "保存", "允许", "允許", "继续", "繼續", "完成", "是",
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
        if summary.actions.iter().any(|action| action == "AXPress") {
            return perform_ax_action(element, "AXPress");
        }
        let point = point_for_element(summary, "dialog button")?;
        post_mouse_click(point, MouseButton::Left)
    }

    fn dialog_op_name(op: MacControlDialogOp) -> &'static str {
        match op {
            MacControlDialogOp::Inspect => "inspect",
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
    ) -> Result<(CfOwned, MacControlElementSummary, MacControlSnapshot), String> {
        let snapshot = capture_ax_snapshot(MacControlSnapshotRequest {
            include_screenshot: false,
            max_elements,
            max_depth,
            ..Default::default()
        })?;
        if !frontmost_app_matches_act_target(&snapshot, target) {
            return Err("Frontmost app did not match the act target.".to_string());
        }
        let summary = snapshot
            .elements
            .iter()
            .filter(|element| element_matches_query(element, target, &snapshot))
            .max_by_key(|element| element_target_score(element, target))
            .cloned()
            .ok_or_else(|| "No AX element matched the act target.".to_string())?;
        let element = resolve_element_by_summary(&summary, max_elements, max_depth)?;
        Ok((element, summary, snapshot))
    }

    fn resolve_type_element(
        target: &MacControlTargetQuery,
        max_elements: usize,
        max_depth: usize,
    ) -> Result<(CfOwned, MacControlElementSummary, MacControlSnapshot), String> {
        let snapshot = capture_ax_snapshot(MacControlSnapshotRequest {
            include_screenshot: false,
            max_elements,
            max_depth,
            ..Default::default()
        })?;
        if !frontmost_app_matches_act_target(&snapshot, target) {
            return Err("Frontmost app did not match the act.type target.".to_string());
        }
        let summary = snapshot
            .elements
            .iter()
            .filter(|element| text_element_matches_query(element, target, &snapshot))
            .max_by_key(|element| type_target_score(element, target))
            .cloned()
            .ok_or_else(|| "No text input element matched the act.type target.".to_string())?;
        let element = resolve_element_by_summary(&summary, max_elements, max_depth)?;
        Ok((element, summary, snapshot))
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

    fn post_mouse_drag(from: CGPoint, to: CGPoint) -> Result<(), String> {
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
        unsafe { CGEventPost(K_CG_HID_EVENT_TAP, down.as_ptr()) };

        for idx in 1..=5 {
            let ratio = f64::from(idx) / 5.0;
            let point = CGPoint {
                x: from.x + (to.x - from.x) * ratio,
                y: from.y + (to.y - from.y) * ratio,
            };
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
            unsafe { CGEventPost(K_CG_HID_EVENT_TAP, dragged.as_ptr()) };
            thread::sleep(Duration::from_millis(20));
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
        unsafe { CGEventPost(K_CG_HID_EVENT_TAP, up.as_ptr()) };
        Ok(())
    }

    fn post_hotkey(keys: &[String]) -> Result<(), String> {
        let mut flags = 0_u64;
        let mut key_code = None;
        for key in keys {
            match key.to_ascii_lowercase().as_str() {
                "cmd" | "command" | "meta" => flags |= K_CG_EVENT_FLAG_MASK_COMMAND,
                "shift" => flags |= K_CG_EVENT_FLAG_MASK_SHIFT,
                "ctrl" | "control" => flags |= K_CG_EVENT_FLAG_MASK_CONTROL,
                "alt" | "option" => flags |= K_CG_EVENT_FLAG_MASK_ALTERNATE,
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
        post_key(key_code, flags)
    }

    fn post_key(key_code: u16, flags: u64) -> Result<(), String> {
        let down = unsafe { CGEventCreateKeyboardEvent(ptr::null(), key_code, true) };
        let up = unsafe { CGEventCreateKeyboardEvent(ptr::null(), key_code, false) };
        let down = CfOwned::new(down as CFTypeRef)
            .ok_or_else(|| "CGEventCreateKeyboardEvent(down) returned null.".to_string())?;
        let up = CfOwned::new(up as CFTypeRef)
            .ok_or_else(|| "CGEventCreateKeyboardEvent(up) returned null.".to_string())?;
        unsafe {
            CGEventSetFlags(down.as_ptr(), flags);
            CGEventSetFlags(up.as_ptr(), flags);
            CGEventPost(K_CG_HID_EVENT_TAP, down.as_ptr());
            CGEventPost(K_CG_HID_EVENT_TAP, up.as_ptr());
        }
        Ok(())
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

    fn menu_item_summary(element: AXUIElementRef, max_depth: usize) -> MacControlMenuItemSummary {
        MacControlMenuItemSummary {
            title: attribute_string(element, "AXTitle"),
            description: attribute_string(element, "AXDescription"),
            value: attribute_string(element, "AXValue"),
            role: attribute_string(element, "AXRole"),
            enabled: attribute_bool(element, "AXEnabled"),
            actions: action_names(element),
            children: menu_children(element, max_depth),
        }
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
            perform_ax_action(child_ref, "AXPress")?;
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

    fn capture_desktop_frame() -> Result<MacControlFramePayload, String> {
        let snapshot_id = ha_core::mac_control::new_snapshot_id();
        let frontmost_app = focused_app_summary();
        let captured = capture_display_frame_bytes(None)?;
        Ok(build_frame_payload(
            &snapshot_id,
            frontmost_app,
            &captured,
            None,
        ))
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
            "row", "search", "sheet", "slider", "tab", "text",
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
