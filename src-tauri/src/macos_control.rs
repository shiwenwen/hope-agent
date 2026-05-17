//! Desktop macOS control bridge.
//!
//! Phase 3 registers the authorized desktop process and exposes Accessibility
//! snapshots, primary-display JPEG frames, app launch/focus, window operations,
//! AX-first element actions, and menu inspection/clicks.

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::{CStr, CString};
    use std::os::raw::{c_char, c_void};
    use std::ptr;
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    use async_trait::async_trait;
    use base64::Engine;
    use ha_core::mac_control::{
        MacControlActOp, MacControlActRequest, MacControlActResult, MacControlAppSummary,
        MacControlAppsOp, MacControlAppsRequest, MacControlAppsResult, MacControlBounds,
        MacControlBridge, MacControlDisplaySummary, MacControlElementSummary,
        MacControlFramePayload, MacControlMenuItemSummary, MacControlMenuOp, MacControlMenuRequest,
        MacControlMenuResult, MacControlRunningApp, MacControlScreenshotSummary,
        MacControlSnapshot, MacControlSnapshotRequest, MacControlTargetQuery,
        MacControlWindowSummary, MacControlWindowsOp, MacControlWindowsRequest,
        MacControlWindowsResult,
    };
    use image::codecs::jpeg::JpegEncoder;
    use objc2::rc::Retained;
    use objc2_app_kit::{
        NSApplicationActivationOptions, NSApplicationActivationPolicy, NSRunningApplication,
        NSWorkspace,
    };
    use objc2_foundation::NSString;
    use xcap::Monitor;

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
    const K_CG_MOUSE_BUTTON_LEFT: u32 = 0;
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

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXUIElementCreateSystemWide() -> AXUIElementRef;
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
        if request.include_screenshot {
            match capture_desktop_frame_with_id(
                &snapshot.snapshot_id,
                snapshot.frontmost_app.clone(),
            ) {
                Ok((frame, screenshot)) => {
                    snapshot.screenshot = Some(screenshot);
                    ha_core::mac_control::emit_frame(&frame);
                }
                Err(error) => snapshot.warnings.push(format!(
                    "Screenshot capture failed; returning AX-only snapshot: {error}"
                )),
            }
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
        Ok(snapshot)
    }

    fn handle_apps(request: MacControlAppsRequest) -> Result<MacControlAppsResult, String> {
        let request = request.clamped();
        let workspace = NSWorkspace::sharedWorkspace();
        let frontmost = workspace
            .frontmostApplication()
            .as_deref()
            .map(running_app_summary);
        let running = workspace.runningApplications().to_vec();
        let mut all_apps = running
            .iter()
            .map(|app| running_app_summary(app))
            .collect::<Vec<_>>();
        if let Some(frontmost) = frontmost.clone() {
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
            || (request.op == MacControlAppsOp::Activate
                && !all_apps
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

        let mut launched = None;
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
            MacControlAppsOp::List | MacControlAppsOp::Frontmost => None,
        };

        Ok(MacControlAppsResult {
            op: request.op,
            frontmost,
            apps,
            activated,
            launched,
        })
    }

    fn handle_windows(
        request: MacControlWindowsRequest,
    ) -> Result<MacControlWindowsResult, String> {
        let request = request.clamped();
        let snapshot = capture_ax_snapshot(MacControlSnapshotRequest {
            include_screenshot: false,
            max_elements: request.max_elements,
            max_depth: request.max_depth,
        })?;
        let mut windows = snapshot.windows.clone();
        let acted_window = if request.op == MacControlWindowsOp::List {
            None
        } else {
            let (window, summary) = resolve_window(&request)?;
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
            frontmost_app: snapshot.frontmost_app,
            windows,
            acted_window,
        })
    }

    fn handle_act(request: MacControlActRequest) -> Result<MacControlActResult, String> {
        let request = request.clamped();
        let mut target = None;
        let execution = match request.op {
            MacControlActOp::Click => {
                if let (Some(x), Some(y)) = (request.x, request.y) {
                    post_mouse_click(CGPoint { x, y })?;
                    "CGEventClick".to_string()
                } else {
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
                        post_mouse_click(CGPoint {
                            x: bounds.x + bounds.width / 2.0,
                            y: bounds.y + bounds.height / 2.0,
                        })?;
                        "CGEventFallback".to_string()
                    }
                }
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
                    let (element, summary, _) =
                        resolve_element(&request.target, request.max_elements, request.max_depth)?;
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
        };
        let snapshot = capture_ax_snapshot(MacControlSnapshotRequest {
            include_screenshot: false,
            max_elements: request.max_elements,
            max_depth: request.max_depth,
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
        let app = focused_app_element()?;
        let menu_bar = copy_attribute(app.as_ptr() as AXUIElementRef, "AXMenuBar")
            .ok_or_else(|| "Focused app does not expose an AXMenuBar.".to_string())?;
        let items = menu_children(menu_bar.as_ptr() as AXUIElementRef, request.max_depth);
        let clicked = if request.op == MacControlMenuOp::Click {
            Some(click_menu_path(
                menu_bar.as_ptr() as AXUIElementRef,
                &request.path,
            )?)
        } else {
            None
        };

        Ok(MacControlMenuResult {
            op: request.op,
            path: request.path,
            items,
            clicked,
        })
    }

    fn app_matches_request(app: &MacControlRunningApp, request: &MacControlAppsRequest) -> bool {
        if request.pid.is_some_and(|pid| app.pid != pid) {
            return false;
        }
        if !contains_ci(app.bundle_id.as_deref(), request.bundle_id.as_deref()) {
            return false;
        }
        if !contains_ci(app.name.as_deref(), request.app_name.as_deref()) {
            return false;
        }
        true
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
        if ok {
            Ok(())
        } else {
            Err("macOS refused the app activation request.".to_string())
        }
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
                .find(|app| app_matches_request(&running_app_summary(app), request))
            {
                return Some(app.clone());
            }
        }

        if let Some(app) = running
            .iter()
            .find(|app| app_matches_request(&running_app_summary(app), request))
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
        let app = focused_app_element()?;
        let windows = copy_attribute(app.as_ptr() as AXUIElementRef, "AXWindows")
            .ok_or_else(|| "Focused app does not expose AXWindows.".to_string())?;
        for (idx, window_ref) in cf_array_values(windows.as_ptr()).into_iter().enumerate() {
            let id = format!("win_{}", idx + 1);
            let summary = window_summary(window_ref as AXUIElementRef, &id);
            if window_matches_request(&summary, request) {
                let retained = unsafe { CFRetain(window_ref as CFTypeRef) };
                let window = CfOwned::new(retained)
                    .ok_or_else(|| "Unable to retain matched AX window.".to_string())?;
                return Ok((window, summary));
            }
        }
        Err("No frontmost-app window matched the request.".to_string())
    }

    fn window_matches_request(
        window: &MacControlWindowSummary,
        request: &MacControlWindowsRequest,
    ) -> bool {
        if request
            .window_id
            .as_deref()
            .filter(|query| !query.is_empty())
            .is_some_and(|query| query != window.id)
        {
            return false;
        }
        contains_ci(
            window.title.as_deref(),
            request.target.window_title.as_deref(),
        )
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
        })?;
        let summary = snapshot
            .elements
            .iter()
            .find(|element| element_matches_query(element, target, &snapshot))
            .cloned()
            .ok_or_else(|| "No AX element matched the act target.".to_string())?;
        let element = resolve_element_by_id(&summary.id, max_elements, max_depth)?;
        Ok((element, summary, snapshot))
    }

    fn resolve_element_by_id(
        element_id: &str,
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
                if let Some(element) = find_element_by_generated_id(
                    window_ref as AXUIElementRef,
                    0,
                    Some(&window_id),
                    &mut state,
                    element_id,
                ) {
                    return Ok(element);
                }
            }
        }
        find_element_by_generated_id(
            app.as_ptr() as AXUIElementRef,
            0,
            None,
            &mut state,
            element_id,
        )
        .ok_or_else(|| "Matched AX element became stale before action.".to_string())
    }

    fn find_element_by_generated_id(
        element: AXUIElementRef,
        depth: usize,
        window_id: Option<&str>,
        state: &mut CaptureState,
        target_id: &str,
    ) -> Option<CfOwned> {
        if state.elements.len() >= state.max_elements {
            state.truncated = true;
            return None;
        }
        let summary = element_summary(element, window_id, state.next_element_id);
        if should_include_element(&summary) {
            state.next_element_id += 1;
            state.elements.push(summary.clone());
            if summary.id == target_id {
                let retained = unsafe { CFRetain(element as CFTypeRef) };
                return CfOwned::new(retained);
            }
            if state.elements.len() >= state.max_elements {
                state.truncated = true;
                return None;
            }
        }
        if depth >= state.max_depth {
            return None;
        }
        let children = copy_attribute(element, "AXChildren")
            .or_else(|| copy_attribute(element, "AXVisibleChildren"))?;
        for child_ref in cf_array_values(children.as_ptr()) {
            if let Some(found) = find_element_by_generated_id(
                child_ref as AXUIElementRef,
                depth + 1,
                window_id,
                state,
                target_id,
            ) {
                return Some(found);
            }
            if state.truncated {
                return None;
            }
        }
        None
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
                    .is_some_and(|window| contains_ci(window.title.as_deref(), Some(query)))
            })
            .unwrap_or(true)
        {
            return false;
        }
        true
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

    fn post_mouse_click(point: CGPoint) -> Result<(), String> {
        let down = unsafe {
            CGEventCreateMouseEvent(
                ptr::null(),
                K_CG_EVENT_LEFT_MOUSE_DOWN,
                point,
                K_CG_MOUSE_BUTTON_LEFT,
            )
        };
        let up = unsafe {
            CGEventCreateMouseEvent(
                ptr::null(),
                K_CG_EVENT_LEFT_MOUSE_UP,
                point,
                K_CG_MOUSE_BUTTON_LEFT,
            )
        };
        let down = CfOwned::new(down as CFTypeRef)
            .ok_or_else(|| "CGEventCreateMouseEvent(down) returned null.".to_string())?;
        let up = CfOwned::new(up as CFTypeRef)
            .ok_or_else(|| "CGEventCreateMouseEvent(up) returned null.".to_string())?;
        unsafe {
            CGEventPost(K_CG_HID_EVENT_TAP, down.as_ptr());
            CGEventPost(K_CG_HID_EVENT_TAP, up.as_ptr());
        }
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
            role: attribute_string(element, "AXRole"),
            enabled: attribute_bool(element, "AXEnabled"),
            children: menu_children(element, max_depth),
        }
    }

    fn click_menu_path(
        menu_root: AXUIElementRef,
        path: &[String],
    ) -> Result<MacControlMenuItemSummary, String> {
        let mut current = menu_root;
        let mut last = None;
        for part in path {
            let child = find_menu_child(current, part)
                .ok_or_else(|| format!("Menu path component '{part}' was not found."))?;
            perform_ax_action(child.as_ptr() as AXUIElementRef, "AXPress")?;
            thread::sleep(Duration::from_millis(120));
            current = child.as_ptr() as AXUIElementRef;
            last = Some(menu_item_summary(current, 2));
        }
        last.ok_or_else(|| "menu.click requires a non-empty path.".to_string())
    }

    fn find_menu_child(element: AXUIElementRef, title: &str) -> Option<CfOwned> {
        let children = copy_attribute(element, "AXChildren")
            .or_else(|| copy_attribute(element, "AXMenuItems"))?;
        for child_ref in cf_array_values(children.as_ptr()) {
            let child = child_ref as AXUIElementRef;
            if contains_ci(attribute_string(child, "AXTitle").as_deref(), Some(title)) {
                let retained = unsafe { CFRetain(child_ref as CFTypeRef) };
                return CfOwned::new(retained);
            }
        }
        None
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

    fn capture_desktop_frame() -> Result<MacControlFramePayload, String> {
        let snapshot_id = ha_core::mac_control::new_snapshot_id();
        let frontmost_app = focused_app_summary();
        let captured = capture_desktop_frame_bytes()?;
        Ok(build_frame_payload(
            &snapshot_id,
            frontmost_app,
            &captured,
            None,
        ))
    }

    fn capture_desktop_frame_with_id(
        snapshot_id: &str,
        frontmost_app: Option<MacControlAppSummary>,
    ) -> Result<(MacControlFramePayload, MacControlScreenshotSummary), String> {
        let captured = capture_desktop_frame_bytes()?;
        let screenshot = ha_core::mac_control::store_screenshot_jpeg(
            snapshot_id,
            &captured.jpeg,
            captured.width_px,
            captured.height_px,
        )?;
        let frame = build_frame_payload(snapshot_id, frontmost_app, &captured, Some(&screenshot));
        Ok((frame, screenshot))
    }

    fn capture_desktop_frame_bytes() -> Result<CapturedDesktopFrame, String> {
        let monitors = Monitor::all().map_err(|e| format!("Failed to list macOS displays: {e}"))?;
        let monitor = monitors
            .iter()
            .find(|monitor| monitor.is_primary().unwrap_or(false))
            .or_else(|| monitors.first())
            .ok_or_else(|| "No macOS displays detected.".to_string())?;
        let rgba_image = monitor.capture_image().map_err(|e| {
            format!("Desktop capture failed; Screen Recording permission may be missing: {e}")
        })?;
        let width_px = rgba_image.width();
        let height_px = rgba_image.height();
        let rgb_image = image::DynamicImage::ImageRgba8(rgba_image).to_rgb8();
        let mut jpeg = Vec::new();
        let mut encoder = JpegEncoder::new_with_quality(&mut jpeg, 70);
        encoder
            .encode_image(&rgb_image)
            .map_err(|e| format!("Failed to encode macOS frame as JPEG: {e}"))?;

        Ok(CapturedDesktopFrame {
            jpeg,
            width_px,
            height_px,
        })
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
            captured_at: chrono::Utc::now().timestamp_millis(),
            frontmost_app,
        }
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

    fn app_summary(app: AXUIElementRef) -> MacControlAppSummary {
        MacControlAppSummary {
            pid: ax_pid(app).unwrap_or_default(),
            bundle_id: None,
            name: attribute_string(app, "AXTitle"),
        }
    }

    fn window_summary(window: AXUIElementRef, id: &str) -> MacControlWindowSummary {
        MacControlWindowSummary {
            id: id.to_string(),
            app_pid: ax_pid(window),
            title: attribute_string(window, "AXTitle"),
            focused: attribute_bool(window, "AXFocused").unwrap_or(false),
            bounds_points: element_bounds(window),
        }
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
            "button", "checkbox", "combobox", "link", "menu", "outline", "pop", "radio", "row",
            "search", "slider", "tab", "text",
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
