use ha_core::app_warn;
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;
use tauri::{LogicalSize, PhysicalPosition, PhysicalSize, Position, Size};

const MAIN_WINDOW_LABEL: &str = "main";
const MIN_MAIN_WINDOW_WIDTH: f64 = 840.0;
const MIN_MAIN_WINDOW_HEIGHT: f64 = 520.0;
const MAX_PERSISTED_DIMENSION: f64 = 10_000.0;
const SAVE_DEBOUNCE_MS: u64 = 300;
const SCREEN_MARGIN: f64 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MainWindowState {
    width: f64,
    height: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    x: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    y: Option<i32>,
}

impl MainWindowState {
    fn new(width: f64, height: f64) -> Option<Self> {
        if !width.is_finite()
            || !height.is_finite()
            || width <= 0.0
            || height <= 0.0
            || width > MAX_PERSISTED_DIMENSION
            || height > MAX_PERSISTED_DIMENSION
        {
            return None;
        }
        Some(Self {
            width: width.round().max(MIN_MAIN_WINDOW_WIDTH),
            height: height.round().max(MIN_MAIN_WINDOW_HEIGHT),
            x: None,
            y: None,
        })
    }

    fn from_window_snapshot(
        size: PhysicalSize<u32>,
        scale_factor: f64,
        position: Option<PhysicalPosition<i32>>,
    ) -> Option<Self> {
        if !scale_factor.is_finite() || scale_factor <= 0.0 {
            return None;
        }
        let mut state = Self::new(
            f64::from(size.width) / scale_factor,
            f64::from(size.height) / scale_factor,
        )?;
        if let Some(position) = position {
            state.x = Some(position.x);
            state.y = Some(position.y);
        }
        Some(state)
    }

    fn clamp_to_monitor(self, monitor: Option<&tauri::Monitor>) -> Self {
        let Some(monitor) = monitor else {
            return self;
        };
        let scale_factor = monitor.scale_factor();
        if !scale_factor.is_finite() || scale_factor <= 0.0 {
            return self;
        }
        let work_area = monitor.work_area();
        let max_width = (f64::from(work_area.size.width) / scale_factor - SCREEN_MARGIN)
            .max(MIN_MAIN_WINDOW_WIDTH);
        let max_height = (f64::from(work_area.size.height) / scale_factor - SCREEN_MARGIN)
            .max(MIN_MAIN_WINDOW_HEIGHT);

        Self {
            width: self.width.min(max_width).max(MIN_MAIN_WINDOW_WIDTH),
            height: self.height.min(max_height).max(MIN_MAIN_WINDOW_HEIGHT),
            x: self.x,
            y: self.y,
        }
    }

    fn as_logical_size(self) -> LogicalSize<f64> {
        LogicalSize::new(self.width, self.height)
    }

    fn saved_position(self) -> Option<PhysicalPosition<i32>> {
        Some(PhysicalPosition::new(self.x?, self.y?))
    }

    fn sanitized(self) -> Option<Self> {
        let mut state = Self::new(self.width, self.height)?;
        if self.x.is_some() && self.y.is_some() {
            state.x = self.x;
            state.y = self.y;
        }
        Some(state)
    }
}

pub(crate) type ResizeSaveToken = Arc<AtomicU64>;

pub(crate) fn new_resize_save_token() -> ResizeSaveToken {
    Arc::new(AtomicU64::new(0))
}

pub(crate) fn restore_main_window_state(app: &tauri::App) {
    use tauri::Manager;

    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };
    if let Err(e) = window.set_min_size(Some(Size::Logical(LogicalSize::new(
        MIN_MAIN_WINDOW_WIDTH,
        MIN_MAIN_WINDOW_HEIGHT,
    )))) {
        app_warn!(
            "window",
            "restore_size",
            "failed to apply window min size: {}",
            e
        );
    }
    let state = match load_state() {
        Ok(Some(state)) => state,
        Ok(None) => return,
        Err(e) => {
            app_warn!(
                "window",
                "restore_size",
                "failed to read window state: {}",
                e
            );
            return;
        }
    };
    let monitor = window.current_monitor().ok().flatten();
    let available_monitors = match window.available_monitors() {
        Ok(monitors) => monitors,
        Err(e) => {
            app_warn!("window", "restore_size", "failed to read monitors: {}", e);
            Vec::new()
        }
    };
    let saved_position = state.saved_position();
    let target_monitor = saved_position
        .as_ref()
        .and_then(|position| monitor_containing_position(&available_monitors, position))
        .or(monitor.as_ref());
    let state = state.clamp_to_monitor(target_monitor);
    if let Err(e) = window.set_size(Size::Logical(state.as_logical_size())) {
        app_warn!(
            "window",
            "restore_size",
            "failed to apply window size: {}",
            e
        );
        return;
    }
    if let Some(position) = saved_position {
        let position = restore_position(position, state, target_monitor, &available_monitors);
        if let Err(e) = window.set_position(Position::Physical(position)) {
            app_warn!(
                "window",
                "restore_size",
                "failed to restore window position: {}",
                e
            );
        }
    } else if let Err(e) = window.center() {
        app_warn!(
            "window",
            "restore_size",
            "failed to center restored window without saved position: {}",
            e
        );
    }
}

pub(crate) fn handle_main_window_event(
    window: &tauri::Window,
    event: &tauri::WindowEvent,
    save_token: &ResizeSaveToken,
) {
    if window.label() != MAIN_WINDOW_LABEL {
        return;
    }

    match event {
        tauri::WindowEvent::Resized(size) => {
            schedule_save_from_snapshot(window, *size, None, save_token.clone());
        }
        tauri::WindowEvent::Moved(_) => {
            schedule_save_from_window(window, save_token.clone());
        }
        tauri::WindowEvent::ScaleFactorChanged {
            scale_factor,
            new_inner_size,
            ..
        } => {
            schedule_save_from_snapshot(
                window,
                *new_inner_size,
                Some(*scale_factor),
                save_token.clone(),
            );
        }
        tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed => {
            save_token.fetch_add(1, Ordering::SeqCst);
            if let Err(e) = save_current_window_state(window) {
                app_warn!("window", "save_state", "failed to save window state: {}", e);
            }
        }
        _ => {}
    }
}

fn schedule_save_from_window(window: &tauri::Window, save_token: ResizeSaveToken) {
    let size = match window.inner_size() {
        Ok(size) => size,
        Err(e) => {
            app_warn!("window", "save_state", "failed to read window size: {}", e);
            return;
        }
    };
    schedule_save_from_snapshot(window, size, None, save_token);
}

fn schedule_save_from_snapshot(
    window: &tauri::Window,
    size: PhysicalSize<u32>,
    scale_factor: Option<f64>,
    save_token: ResizeSaveToken,
) {
    if !should_persist_state(window) {
        return;
    }
    let scale_factor = match scale_factor {
        Some(value) => value,
        None => match window.scale_factor() {
            Ok(value) => value,
            Err(e) => {
                app_warn!("window", "save_state", "failed to read scale factor: {}", e);
                return;
            }
        },
    };
    let position = match window.outer_position() {
        Ok(position) => Some(position),
        Err(e) => {
            app_warn!(
                "window",
                "save_state",
                "failed to read window position: {}",
                e
            );
            None
        }
    };
    let Some(state) = MainWindowState::from_window_snapshot(size, scale_factor, position) else {
        return;
    };
    let generation = save_token.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(SAVE_DEBOUNCE_MS)).await;
        if save_token.load(Ordering::SeqCst) != generation {
            return;
        }
        if let Err(e) = save_state(state) {
            app_warn!(
                "window",
                "save_state",
                "failed to persist window state: {}",
                e
            );
        }
    });
}

fn save_current_window_state(window: &tauri::Window) -> anyhow::Result<()> {
    if !should_persist_state(window) {
        return Ok(());
    }
    let size = window.inner_size()?;
    let scale_factor = window.scale_factor()?;
    let position = window.outer_position().ok();
    if let Some(state) = MainWindowState::from_window_snapshot(size, scale_factor, position) {
        save_state(state)?;
    }
    Ok(())
}

fn monitor_containing_position<'a>(
    monitors: &'a [tauri::Monitor],
    position: &PhysicalPosition<i32>,
) -> Option<&'a tauri::Monitor> {
    monitors
        .iter()
        .find(|monitor| position_is_in_work_area(position, monitor))
}

fn position_is_in_work_area(position: &PhysicalPosition<i32>, monitor: &tauri::Monitor) -> bool {
    let work_area = monitor.work_area();
    let left = i64::from(work_area.position.x);
    let top = i64::from(work_area.position.y);
    let right = left + i64::from(work_area.size.width);
    let bottom = top + i64::from(work_area.size.height);
    let x = i64::from(position.x);
    let y = i64::from(position.y);

    x >= left && x < right && y >= top && y < bottom
}

fn restore_position(
    position: PhysicalPosition<i32>,
    state: MainWindowState,
    target_monitor: Option<&tauri::Monitor>,
    available_monitors: &[tauri::Monitor],
) -> PhysicalPosition<i32> {
    if available_monitors
        .iter()
        .any(|monitor| window_rect_is_in_work_area(&position, state, monitor))
    {
        return position;
    }

    let Some(monitor) = target_monitor.or_else(|| available_monitors.first()) else {
        return position;
    };
    clamp_position_to_monitor(position, state, monitor)
}

fn window_rect_is_in_work_area(
    position: &PhysicalPosition<i32>,
    state: MainWindowState,
    monitor: &tauri::Monitor,
) -> bool {
    let scale_factor = monitor.scale_factor();
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return false;
    }
    let width = logical_dimension_to_physical(state.width, scale_factor);
    let height = logical_dimension_to_physical(state.height, scale_factor);
    let work_area = monitor.work_area();
    rect_fits_in_area(position, width, height, work_area.position, work_area.size)
}

fn rect_fits_in_area(
    position: &PhysicalPosition<i32>,
    width: i32,
    height: i32,
    area_position: PhysicalPosition<i32>,
    area_size: PhysicalSize<u32>,
) -> bool {
    let left = i64::from(area_position.x);
    let top = i64::from(area_position.y);
    let right = left + i64::from(area_size.width);
    let bottom = top + i64::from(area_size.height);
    let x = i64::from(position.x);
    let y = i64::from(position.y);
    let window_right = x + i64::from(width.max(1));
    let window_bottom = y + i64::from(height.max(1));

    x >= left && y >= top && window_right <= right && window_bottom <= bottom
}

fn clamp_position_to_monitor(
    position: PhysicalPosition<i32>,
    state: MainWindowState,
    monitor: &tauri::Monitor,
) -> PhysicalPosition<i32> {
    let scale_factor = monitor.scale_factor();
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return position;
    }
    let width = logical_dimension_to_physical(state.width, scale_factor);
    let height = logical_dimension_to_physical(state.height, scale_factor);
    let work_area = monitor.work_area();

    PhysicalPosition::new(
        clamp_axis(
            position.x,
            work_area.position.x,
            work_area.size.width,
            width,
        ),
        clamp_axis(
            position.y,
            work_area.position.y,
            work_area.size.height,
            height,
        ),
    )
}

fn logical_dimension_to_physical(value: f64, scale_factor: f64) -> i32 {
    (value * scale_factor)
        .round()
        .clamp(1.0, f64::from(i32::MAX)) as i32
}

fn clamp_axis(value: i32, start: i32, area_span: u32, window_span: i32) -> i32 {
    let min = i64::from(start);
    let max = min + i64::from(area_span) - i64::from(window_span.max(1));
    let value = i64::from(value);
    let clamped = if max >= min {
        value.clamp(min, max)
    } else {
        min
    };
    clamped.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

fn should_persist_state(window: &tauri::Window) -> bool {
    if window.is_fullscreen().unwrap_or(false)
        || window.is_minimized().unwrap_or(false)
        || window.is_maximized().unwrap_or(false)
    {
        return false;
    }
    true
}

fn load_state() -> anyhow::Result<Option<MainWindowState>> {
    let path = ha_core::paths::window_state_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let data = std::fs::read_to_string(path)?;
    let state: MainWindowState = serde_json::from_str(&data)?;
    Ok(state.sanitized())
}

fn save_state(state: MainWindowState) -> anyhow::Result<()> {
    let path = ha_core::paths::window_state_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp_path = path.with_extension("json.tmp");
    let data = serde_json::to_string_pretty(&state)?;
    std::fs::write(&temp_path, data)?;
    std::fs::rename(temp_path, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_size_is_sanitized() {
        assert_eq!(
            MainWindowState::new(900.4, 700.6),
            Some(MainWindowState {
                width: 900.0,
                height: 701.0,
                x: None,
                y: None,
            })
        );
        assert_eq!(
            MainWindowState::new(100.0, 100.0),
            Some(MainWindowState {
                width: MIN_MAIN_WINDOW_WIDTH,
                height: MIN_MAIN_WINDOW_HEIGHT,
                x: None,
                y: None,
            })
        );
        assert_eq!(MainWindowState::new(f64::NAN, 700.0), None);
        assert_eq!(
            MainWindowState::new(MAX_PERSISTED_DIMENSION + 1.0, 700.0),
            None
        );
    }

    #[test]
    fn physical_size_uses_scale_factor() {
        assert_eq!(
            MainWindowState::from_window_snapshot(PhysicalSize::new(2720, 1720), 2.0, None),
            Some(MainWindowState {
                width: 1360.0,
                height: 860.0,
                x: None,
                y: None,
            })
        );
        assert_eq!(
            MainWindowState::from_window_snapshot(PhysicalSize::new(1, 1), 0.0, None),
            None
        );
    }

    #[test]
    fn legacy_state_without_position_still_loads() {
        let state: MainWindowState =
            serde_json::from_str(r#"{"width":900.0,"height":700.0}"#).unwrap();

        assert_eq!(
            state.sanitized(),
            Some(MainWindowState {
                width: 900.0,
                height: 700.0,
                x: None,
                y: None,
            })
        );
    }

    #[test]
    fn physical_position_is_preserved() {
        let position = PhysicalPosition::new(-1200, 80);
        let state = MainWindowState::from_window_snapshot(
            PhysicalSize::new(1800, 1200),
            2.0,
            Some(position),
        )
        .unwrap();

        assert_eq!(state.saved_position(), Some(position));
        assert_eq!(
            state.sanitized().and_then(MainWindowState::saved_position),
            Some(position)
        );
    }

    #[test]
    fn rect_fit_requires_entire_window_inside_area() {
        let area_position = PhysicalPosition::new(0, 0);
        let area_size = PhysicalSize::new(1000, 800);

        assert!(rect_fits_in_area(
            &PhysicalPosition::new(100, 100),
            600,
            400,
            area_position,
            area_size,
        ));
        assert!(!rect_fits_in_area(
            &PhysicalPosition::new(900, 700),
            300,
            200,
            area_position,
            area_size,
        ));
    }
}
