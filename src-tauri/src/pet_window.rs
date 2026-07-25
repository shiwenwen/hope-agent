use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
#[cfg(not(target_os = "macos"))]
use tauri::Size;
use tauri::{Emitter, Manager, PhysicalPosition, Position, WebviewWindow};

const PET_WINDOW_LABEL: &str = "pet";
const PET_ONLY_WIDTH: f64 = 120.0;
const PET_ONLY_HEIGHT: f64 = 128.0;
const PET_ONLY_ANCHOR_X: f64 = 60.0;
const PET_ONLY_ANCHOR_Y: f64 = 116.0;
const SAFE_MARGIN_LOGICAL: f64 = 12.0;

static MOVE_SAVE_REVISION: AtomicU64 = AtomicU64::new(0);
static MOVE_SAVE_WORKER_RUNNING: AtomicBool = AtomicBool::new(false);
static BOUNDS_APPLY_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
#[cfg(target_os = "macos")]
static POINTER_MONITORS_INSTALLED: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "macos")]
static POINTER_WAS_INSIDE: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "macos")]
static POINTER_LAST_EMIT_MS: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "macos")]
static DRAG_RELEASE_TRACKER_RUNNING: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "macos")]
const PET_INACTIVE_POINTER_EVENT: &str = "pet:inactive_pointer";
#[cfg(target_os = "macos")]
const PET_NATIVE_DRAG_ENDED_EVENT: &str = "pet:native_drag_ended";

#[derive(Debug)]
struct PetLayoutState {
    revision: u64,
    anchor: (f64, f64),
}

static LAYOUT_STATE: OnceLock<Mutex<PetLayoutState>> = OnceLock::new();
static SYNC_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn layout_state() -> &'static Mutex<PetLayoutState> {
    LAYOUT_STATE.get_or_init(|| {
        Mutex::new(PetLayoutState {
            revision: 0,
            anchor: (PET_ONLY_ANCHOR_X, PET_ONLY_ANCHOR_Y),
        })
    })
}

fn bounds_apply_lock() -> &'static tokio::sync::Mutex<()> {
    BOUNDS_APPLY_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetWindowBoundsRequest {
    pub layout_revision: u64,
    pub width: f64,
    pub height: f64,
    pub previous_anchor_x: f64,
    pub previous_anchor_y: f64,
    pub next_anchor_x: f64,
    pub next_anchor_y: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PetWindowBoundsResult {
    pub applied: bool,
    pub layout_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PetWindowState {
    schema_version: u32,
    monitor_name: Option<String>,
    #[serde(default)]
    work_area: Option<SavedWorkArea>,
    #[serde(default)]
    scale_factor: Option<f64>,
    anchor_x: f64,
    anchor_y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedWorkArea {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
struct PetInactivePointerEvent {
    inside: bool,
    x: f64,
    y: f64,
}

#[cfg(target_os = "macos")]
fn emit_inactive_pointer_leave(app: &tauri::AppHandle) {
    if !POINTER_WAS_INSIDE.swap(false, Ordering::AcqRel) {
        return;
    }
    if let Some(window) = app.get_webview_window(PET_WINDOW_LABEL) {
        let _ = window.emit(
            PET_INACTIVE_POINTER_EVENT,
            PetInactivePointerEvent {
                inside: false,
                x: 0.0,
                y: 0.0,
            },
        );
    }
}

#[cfg(target_os = "macos")]
fn forward_inactive_pointer(app: &tauri::AppHandle, timestamp: f64) {
    use objc2_app_kit::{NSEvent, NSWindow};

    let Some(window) = app.get_webview_window(PET_WINDOW_LABEL) else {
        POINTER_WAS_INSIDE.store(false, Ordering::Release);
        return;
    };
    let Ok(raw_window) = window.ns_window() else {
        emit_inactive_pointer_leave(app);
        return;
    };
    let ns_window: &NSWindow = unsafe { &*raw_window.cast() };
    let frame = ns_window.frame();
    let pointer = NSEvent::mouseLocation();
    let inside = pointer.x >= frame.origin.x
        && pointer.x < frame.origin.x + frame.size.width
        && pointer.y >= frame.origin.y
        && pointer.y < frame.origin.y + frame.size.height;
    let was_inside = POINTER_WAS_INSIDE.swap(inside, Ordering::AcqRel);
    if !inside {
        if was_inside {
            let _ = window.emit(
                PET_INACTIVE_POINTER_EVENT,
                PetInactivePointerEvent {
                    inside: false,
                    x: 0.0,
                    y: 0.0,
                },
            );
        }
        return;
    }

    // Global mouse monitors can fire much faster than the display refresh
    // rate. Cap bridge traffic at ~30Hz while preserving the first enter.
    let timestamp_ms = (timestamp.max(0.0) * 1000.0) as u64;
    let previous_ms = POINTER_LAST_EMIT_MS.load(Ordering::Acquire);
    if was_inside && timestamp_ms.saturating_sub(previous_ms) < 33 {
        return;
    }
    POINTER_LAST_EMIT_MS.store(timestamp_ms, Ordering::Release);
    let _ = window.emit(
        PET_INACTIVE_POINTER_EVENT,
        PetInactivePointerEvent {
            inside: true,
            x: pointer.x - frame.origin.x,
            y: frame.origin.y + frame.size.height - pointer.y,
        },
    );
}

#[cfg(target_os = "macos")]
fn track_native_drag_release(app: &tauri::AppHandle) {
    use objc2_app_kit::NSEvent;

    if DRAG_RELEASE_TRACKER_RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        while NSEvent::pressedMouseButtons() & 1 != 0 {
            std::thread::sleep(Duration::from_millis(16));
        }
        DRAG_RELEASE_TRACKER_RUNNING.store(false, Ordering::Release);
        if let Some(window) = app.get_webview_window(PET_WINDOW_LABEL) {
            let _ = window.emit(PET_NATIVE_DRAG_ENDED_EVENT, ());
        }
    });
}

#[cfg(target_os = "macos")]
fn handle_pointer_monitor_event(app: &tauri::AppHandle, event: &objc2_app_kit::NSEvent) {
    use objc2_app_kit::NSEventType;

    forward_inactive_pointer(app, event.timestamp());
    if event.r#type() == NSEventType::LeftMouseDragged && POINTER_WAS_INSIDE.load(Ordering::Acquire)
    {
        track_native_drag_release(app);
    }
}

#[cfg(target_os = "macos")]
fn install_inactive_pointer_monitors(app: &tauri::AppHandle) {
    use std::ptr::NonNull;

    use block2::{DynBlock, RcBlock};
    use objc2::rc::Retained;
    use objc2_app_kit::{NSEvent, NSEventMask};

    let event_mask = NSEventMask::MouseMoved | NSEventMask::LeftMouseDragged;

    if POINTER_MONITORS_INSTALLED.swap(true, Ordering::AcqRel) {
        return;
    }

    let global_app = app.clone();
    let global_handler = RcBlock::new(move |event: NonNull<NSEvent>| {
        handle_pointer_monitor_event(&global_app, unsafe { event.as_ref() });
    });
    let global_handler: &DynBlock<dyn Fn(NonNull<NSEvent>)> = &global_handler;
    let Some(global_monitor) =
        NSEvent::addGlobalMonitorForEventsMatchingMask_handler(event_mask, global_handler)
    else {
        POINTER_MONITORS_INSTALLED.store(false, Ordering::Release);
        ha_core::app_warn!(
            "pet",
            "inactive_pointer",
            "Failed to install the macOS global mouse-move monitor"
        );
        return;
    };
    // One process-lifetime monitor is reused across PetWindow recreation. The
    // handler resolves the current `pet` label on every event, so it never
    // retains a closed NSWindow pointer.
    let _ = Retained::into_raw(global_monitor);

    // A global monitor intentionally excludes events sent to this app. Use
    // the same coordinate bridge for local events as well: the main Hope
    // window can be active while PetWindow remains non-key, and clearing hover
    // on every such move makes the actions alternate visible/hidden. Return
    // the original event unchanged after projecting its position.
    let local_app = app.clone();
    let local_handler = RcBlock::new(move |event: NonNull<NSEvent>| {
        handle_pointer_monitor_event(&local_app, unsafe { event.as_ref() });
        event.as_ptr()
    });
    let local_handler: &DynBlock<dyn Fn(NonNull<NSEvent>) -> *mut NSEvent> = &local_handler;
    let local_monitor =
        unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(event_mask, local_handler) };
    if let Some(local_monitor) = local_monitor {
        let _ = Retained::into_raw(local_monitor);
    } else {
        ha_core::app_warn!(
            "pet",
            "inactive_pointer",
            "Failed to install the macOS local mouse-move monitor"
        );
    }
}

fn valid_dimension(value: f64, min: f64, max: f64) -> bool {
    value.is_finite() && value >= min && value <= max
}

fn monitor_scale(monitor: &tauri::Monitor) -> f64 {
    let scale = monitor.scale_factor();
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

fn clamp_position_to_monitor(
    monitor: &tauri::Monitor,
    scale: f64,
    x: i32,
    y: i32,
    width_physical: i32,
    height_physical: i32,
) -> (i32, i32) {
    let safe_margin = (SAFE_MARGIN_LOGICAL * scale).round() as i32;
    let area = monitor.work_area();
    let left = area.position.x + safe_margin;
    let top = area.position.y + safe_margin;
    let right = area.position.x + area.size.width as i32 - safe_margin;
    let bottom = area.position.y + area.size.height as i32 - safe_margin;
    (
        x.clamp(left, (right - width_physical).max(left)),
        y.clamp(top, (bottom - height_physical).max(top)),
    )
}

fn clamp_position(
    window: &WebviewWindow,
    x: i32,
    y: i32,
    width_physical: i32,
    height_physical: i32,
) -> (i32, i32) {
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return (x, y);
    };
    let scale = monitor_scale(&monitor);
    clamp_position_to_monitor(&monitor, scale, x, y, width_physical, height_physical)
}

#[cfg(target_os = "macos")]
async fn apply_native_bounds(
    window: &WebviewWindow,
    current_position: PhysicalPosition<i32>,
    next_position: PhysicalPosition<i32>,
    width_physical: i32,
    height_physical: i32,
    scale: f64,
) -> anyhow::Result<()> {
    // `set_size` followed by `set_position` exposes an intermediate native
    // frame to WindowServer. During that frame the webview has its new size at
    // the old origin, so the pet visibly jumps to the top-left before the
    // second operation lands. Convert Tauri's top-left physical delta to
    // AppKit's bottom-left logical coordinates and update the complete frame
    // with one non-animated AppKit operation.
    let delta_x = (f64::from(next_position.x) - f64::from(current_position.x)) / scale;
    let delta_y = (f64::from(next_position.y) - f64::from(current_position.y)) / scale;
    let width = f64::from(width_physical) / scale;
    let height = f64::from(height_physical) / scale;
    let (applied_tx, applied_rx) = tokio::sync::oneshot::channel();
    window.with_webview(move |webview| unsafe {
        use objc2_foundation::{NSPoint, NSRect, NSSize};

        let ns_window: &objc2_app_kit::NSWindow = &*webview.ns_window().cast();
        let current_frame = ns_window.frame();
        let current_top = current_frame.origin.y + current_frame.size.height;
        let next_frame = NSRect::new(
            NSPoint::new(
                current_frame.origin.x + delta_x,
                current_top - delta_y - height,
            ),
            NSSize::new(width, height),
        );
        ns_window.setFrame_display(next_frame, true);
        let _ = applied_tx.send(());
    })?;
    // `with_webview` schedules work on the AppKit thread. Do not acknowledge
    // the IPC request until that work has actually run; React keeps the
    // overlay transparent until this future resolves.
    tokio::time::timeout(Duration::from_secs(2), applied_rx)
        .await
        .map_err(|_| anyhow::anyhow!("pet_window_bounds_apply_timeout"))?
        .map_err(|_| anyhow::anyhow!("pet_window_bounds_apply_cancelled"))?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
async fn apply_native_bounds(
    window: &WebviewWindow,
    current_position: PhysicalPosition<i32>,
    next_position: PhysicalPosition<i32>,
    width_physical: i32,
    height_physical: i32,
    scale: f64,
) -> anyhow::Result<()> {
    let previous_size = window.inner_size()?;
    window.set_size(Size::Physical(tauri::PhysicalSize::new(
        width_physical as u32,
        height_physical as u32,
    )))?;
    if let Err(error) = window.set_position(Position::Physical(next_position)) {
        // Other platforms do not expose a shared atomic geometry primitive
        // here. Keep their previous rollback behavior if the move fails.
        let _ = window.set_size(Size::Physical(previous_size));
        let _ = window.set_position(Position::Physical(current_position));
        return Err(error.into());
    }
    let _ = scale;
    Ok(())
}

pub(crate) async fn apply_bounds(
    window: &WebviewWindow,
    request: PetWindowBoundsRequest,
) -> anyhow::Result<PetWindowBoundsResult> {
    if !valid_dimension(request.width, 112.0, 440.0)
        || !valid_dimension(request.height, 120.0, 640.0)
        || !request.previous_anchor_x.is_finite()
        || !request.previous_anchor_y.is_finite()
        || !request.next_anchor_x.is_finite()
        || !request.next_anchor_y.is_finite()
    {
        anyhow::bail!("pet_window_bounds_invalid");
    }
    // Keep revision ordering and the native callback under one async lock.
    // Commands may overlap; an atomic revision alone can let an older resize
    // finish after a newer one and visually roll the window back.
    let _apply_guard = bounds_apply_lock().lock().await;
    let committed_anchor = {
        let mut state = layout_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if request.layout_revision <= state.revision {
            return Ok(PetWindowBoundsResult {
                applied: false,
                layout_revision: state.revision,
            });
        }
        state.revision = request.layout_revision;
        state.anchor
    };

    let scale = window.scale_factor()?;
    let position = window.outer_position()?;
    // Native geometry is the source of truth. A ResizeObserver can supersede a
    // renderer request after that request has already reached this serialized
    // section. The renderer then intentionally ignores the old reply, so its
    // previous anchor can be stale while `state.anchor` reflects the move that
    // actually happened. Keeping this calculation native prevents rapid
    // bubble or font-size changes from accumulating position drift.
    let anchor_screen_x = position.x as f64 + committed_anchor.0 * scale;
    let anchor_screen_y = position.y as f64 + committed_anchor.1 * scale;
    let width_physical = (request.width * scale).round() as i32;
    let height_physical = (request.height * scale).round() as i32;
    let next_x = (anchor_screen_x - request.next_anchor_x * scale).round() as i32;
    let next_y = (anchor_screen_y - request.next_anchor_y * scale).round() as i32;
    let (next_x, next_y) = clamp_position(window, next_x, next_y, width_physical, height_physical);

    apply_native_bounds(
        window,
        position,
        PhysicalPosition::new(next_x, next_y),
        width_physical,
        height_physical,
        scale,
    )
    .await?;
    let mut state = layout_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.anchor = (request.next_anchor_x, request.next_anchor_y);
    Ok(PetWindowBoundsResult {
        applied: true,
        layout_revision: request.layout_revision,
    })
}

fn restore_position(window: &WebviewWindow) {
    let state = crate::paths::pet_window_state_path()
        .ok()
        .and_then(|path| std::fs::read(path).ok())
        .filter(|bytes| bytes.len() <= 16 * 1024)
        .and_then(|bytes| serde_json::from_slice::<PetWindowState>(&bytes).ok())
        .filter(|state| {
            state.schema_version == 1
                && state.anchor_x.is_finite()
                && state.anchor_y.is_finite()
                && (0.0..=1.0).contains(&state.anchor_x)
                && (0.0..=1.0).contains(&state.anchor_y)
        });
    let monitors = window.available_monitors().unwrap_or_default();
    let monitor = state
        .as_ref()
        .and_then(|state| {
            state.monitor_name.as_ref().and_then(|name| {
                monitors
                    .iter()
                    .filter(|monitor| monitor.name() == Some(name))
                    .min_by_key(|monitor| {
                        let Some(saved) = state.work_area.as_ref() else {
                            return 0_i64;
                        };
                        let area = monitor.work_area();
                        i64::from((area.position.x - saved.x).abs())
                            + i64::from((area.position.y - saved.y).abs())
                            + (i64::from(area.size.width) - i64::from(saved.width)).abs()
                            + (i64::from(area.size.height) - i64::from(saved.height)).abs()
                    })
                    .cloned()
            })
        })
        .or_else(|| window.primary_monitor().ok().flatten())
        .or_else(|| monitors.into_iter().next());
    let Some(monitor) = monitor else { return };
    let area = monitor.work_area();
    // A newly-created window initially reports the primary monitor's scale.
    // Restore geometry in the selected monitor's coordinate system instead;
    // mixed-DPI layouts otherwise shift and clamp pets away from their saved
    // anchor before the window has ever reached that monitor.
    let scale = monitor_scale(&monitor);
    let (anchor_x, anchor_y) = state
        .map(|state| (state.anchor_x, state.anchor_y))
        .unwrap_or((0.92, 0.88));
    let screen_x = area.position.x as f64 + area.size.width as f64 * anchor_x;
    let screen_y = area.position.y as f64 + area.size.height as f64 * anchor_y;
    let x = (screen_x - PET_ONLY_ANCHOR_X * scale).round() as i32;
    let y = (screen_y - PET_ONLY_ANCHOR_Y * scale).round() as i32;
    let (x, y) = clamp_position_to_monitor(
        &monitor,
        scale,
        x,
        y,
        (PET_ONLY_WIDTH * scale).round() as i32,
        (PET_ONLY_HEIGHT * scale).round() as i32,
    );
    let _ = window.set_position(Position::Physical(PhysicalPosition::new(x, y)));
}

pub(crate) fn ensure_window(app: &tauri::AppHandle) -> anyhow::Result<WebviewWindow> {
    if let Some(window) = app.get_webview_window(PET_WINDOW_LABEL) {
        window.show()?;
        return Ok(window);
    }
    {
        let mut state = layout_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.revision = 0;
        state.anchor = (PET_ONLY_ANCHOR_X, PET_ONLY_ANCHOR_Y);
    }
    let url = tauri::WebviewUrl::App("index.html?window=pet".into());
    let window = tauri::WebviewWindowBuilder::new(app, PET_WINDOW_LABEL, url)
        .title("Hope Pet")
        .inner_size(PET_ONLY_WIDTH, PET_ONLY_HEIGHT)
        .min_inner_size(112.0, 120.0)
        .max_inner_size(440.0, 640.0)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .shadow(false)
        .transparent(true)
        .accept_first_mouse(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build()?;

    #[cfg(target_os = "macos")]
    {
        let pointer_app = app.clone();
        let _ = window.with_webview(move |webview| unsafe {
            use objc2::AnyThread;
            use objc2_app_kit::{NSTrackingArea, NSTrackingAreaOptions, NSView};
            use objc2_foundation::{NSPoint, NSRect, NSSize};

            let ns_window: &objc2_app_kit::NSWindow = &*webview.ns_window().cast();
            let ns_webview: &NSView = &*webview.inner().cast();
            let clear =
                objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(0.0, 0.0, 0.0, 0.0);
            ns_window.setBackgroundColor(Some(&clear));
            // `accept_first_mouse` only changes click activation on WRY's
            // WKWebView. Keep both the NSWindow event gate and an always-active
            // WKWebView tracking area enabled for native tracking. WebKit still
            // suppresses DOM hover while another application owns focus, so
            // the process-wide coordinate bridge below supplies declarative
            // hover state without making the pet key or activating Hope Agent.
            ns_window.setAcceptsMouseMovedEvents(true);
            let tracking_options = NSTrackingAreaOptions::MouseEnteredAndExited
                | NSTrackingAreaOptions::MouseMoved
                | NSTrackingAreaOptions::ActiveAlways
                | NSTrackingAreaOptions::InVisibleRect;
            let tracking_area = NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
                tracking_options,
                Some(ns_webview),
                None,
            );
            ns_webview.addTrackingArea(&tracking_area);
            ns_window.setLevel(objc2_app_kit::NSWindowLevel::from(3_isize));
            ns_window.setCollectionBehavior(
                objc2_app_kit::NSWindowCollectionBehavior::CanJoinAllSpaces
                    | objc2_app_kit::NSWindowCollectionBehavior::FullScreenAuxiliary,
            );
            install_inactive_pointer_monitors(&pointer_app);
        });
    }

    restore_position(&window);
    window.show()?;
    Ok(window)
}

pub(crate) fn sync_enabled(app: &tauri::AppHandle, enabled: bool) -> anyhow::Result<()> {
    // Settings commands and the main renderer's config-event bridge can arrive
    // at nearly the same time. Serialize lifecycle changes so two callers
    // cannot both observe a missing label and race to create `pet`.
    let _guard = SYNC_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if enabled {
        ensure_window(app)?;
    } else if let Some(window) = app.get_webview_window(PET_WINDOW_LABEL) {
        window.close()?;
    }
    Ok(())
}

fn persist_state(window: &tauri::Window) -> anyhow::Result<()> {
    let position = window.outer_position()?;
    let monitor = window
        .current_monitor()?
        .or(window.primary_monitor()?)
        .ok_or_else(|| anyhow::anyhow!("pet_monitor_unavailable"))?;
    let scale = monitor_scale(&monitor);
    let area = monitor.work_area();
    let (anchor_x, anchor_y) = layout_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .anchor;
    let screen_x = position.x as f64 + anchor_x * scale;
    let screen_y = position.y as f64 + anchor_y * scale;
    let normalized_x =
        ((screen_x - area.position.x as f64) / area.size.width as f64).clamp(0.0, 1.0);
    let normalized_y =
        ((screen_y - area.position.y as f64) / area.size.height as f64).clamp(0.0, 1.0);
    let state = PetWindowState {
        schema_version: 1,
        monitor_name: monitor.name().cloned(),
        work_area: Some(SavedWorkArea {
            x: area.position.x,
            y: area.position.y,
            width: area.size.width,
            height: area.size.height,
        }),
        scale_factor: Some(scale),
        anchor_x: normalized_x,
        anchor_y: normalized_y,
    };
    let bytes = serde_json::to_vec_pretty(&state)?;
    let path = crate::paths::pet_window_state_path()?;
    Ok(crate::platform::write_atomic(&path, &bytes)?)
}

pub(crate) fn handle_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if window.label() != PET_WINDOW_LABEL || !matches!(event, tauri::WindowEvent::Moved(_)) {
        return;
    }
    MOVE_SAVE_REVISION.fetch_add(1, Ordering::Release);
    if MOVE_SAVE_WORKER_RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let window = window.clone();
    std::thread::spawn(move || loop {
        let observed = MOVE_SAVE_REVISION.load(Ordering::Acquire);
        std::thread::sleep(Duration::from_millis(300));
        if MOVE_SAVE_REVISION.load(Ordering::Acquire) != observed {
            continue;
        }
        if let Err(error) = persist_state(&window) {
            ha_core::app_warn!(
                "pet",
                "window_state",
                "Failed to persist pet position: {}",
                error
            );
        }
        MOVE_SAVE_WORKER_RUNNING.store(false, Ordering::Release);
        if MOVE_SAVE_REVISION.load(Ordering::Acquire) == observed {
            break;
        }
        if MOVE_SAVE_WORKER_RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            break;
        }
    });
}
