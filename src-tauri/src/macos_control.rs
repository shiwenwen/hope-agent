//! Desktop macOS control bridge.
//!
//! Phase 3A registers the authorized desktop process and exposes read-only
//! Accessibility snapshots, primary-display JPEG frames, and low-risk
//! running-app focus control. Pointer/keyboard/window mutations are
//! intentionally left for later slices.

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::{CStr, CString};
    use std::os::raw::{c_char, c_void};
    use std::ptr;
    use std::sync::Arc;

    use async_trait::async_trait;
    use base64::Engine;
    use ha_core::mac_control::{
        MacControlAppSummary, MacControlAppsOp, MacControlAppsRequest, MacControlAppsResult,
        MacControlBounds, MacControlBridge, MacControlDisplaySummary, MacControlElementSummary,
        MacControlFramePayload, MacControlRunningApp, MacControlScreenshotSummary,
        MacControlSnapshot, MacControlSnapshotRequest, MacControlWindowSummary,
    };
    use image::codecs::jpeg::JpegEncoder;
    use objc2_app_kit::{
        NSApplicationActivationOptions, NSApplicationActivationPolicy, NSRunningApplication,
        NSWorkspace,
    };
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

    const AX_ERROR_SUCCESS: AXError = 0;
    const K_CFSTRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const K_AXVALUE_CGPOINT_TYPE: i32 = 1;
    const K_AXVALUE_CGSIZE_TYPE: i32 = 2;
    const K_AXVALUE_CGRECT_TYPE: i32 = 3;

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
        fn AXValueGetType(value: AXValueRef) -> i32;
        fn AXValueGetValue(value: AXValueRef, value_type: i32, value: *mut c_void) -> Boolean;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: CFTypeRef);
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
        let mut apps = running
            .iter()
            .map(|app| running_app_summary(app))
            .filter(|app| app_matches_request(app, &request))
            .take(request.limit)
            .collect::<Vec<_>>();

        let activated = if request.op == MacControlAppsOp::Activate {
            let app = running
                .iter()
                .find(|app| app_matches_request(&running_app_summary(app), &request))
                .ok_or_else(|| "No running macOS app matched the activate request.".to_string())?;
            let ok = app.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
            if !ok {
                return Err("macOS refused the app activation request.".to_string());
            }
            let summary = running_app_summary(app);
            if apps.iter().all(|item| item.pid != summary.pid) {
                apps.insert(0, summary.clone());
            }
            Some(summary)
        } else {
            None
        };

        Ok(MacControlAppsResult {
            op: request.op,
            frontmost,
            apps,
            activated,
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
