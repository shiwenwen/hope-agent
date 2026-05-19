use ha_core::app_warn;
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;
use tauri::{LogicalSize, PhysicalSize, Size};

const MAIN_WINDOW_LABEL: &str = "main";
const MIN_MAIN_WINDOW_WIDTH: f64 = 840.0;
const MIN_MAIN_WINDOW_HEIGHT: f64 = 480.0;
const MAX_PERSISTED_DIMENSION: f64 = 10_000.0;
const SAVE_DEBOUNCE_MS: u64 = 300;
const SCREEN_MARGIN: f64 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MainWindowState {
    width: f64,
    height: f64,
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
        })
    }

    fn from_physical(size: PhysicalSize<u32>, scale_factor: f64) -> Option<Self> {
        if !scale_factor.is_finite() || scale_factor <= 0.0 {
            return None;
        }
        Self::new(
            f64::from(size.width) / scale_factor,
            f64::from(size.height) / scale_factor,
        )
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
        }
    }

    fn as_logical_size(self) -> LogicalSize<f64> {
        LogicalSize::new(self.width, self.height)
    }
}

pub(crate) type ResizeSaveToken = Arc<AtomicU64>;

pub(crate) fn new_resize_save_token() -> ResizeSaveToken {
    Arc::new(AtomicU64::new(0))
}

pub(crate) fn restore_main_window_size(app: &tauri::App) {
    use tauri::Manager;

    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };
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
    let state = state.clamp_to_monitor(monitor.as_ref());
    if let Err(e) = window.set_size(Size::Logical(state.as_logical_size())) {
        app_warn!(
            "window",
            "restore_size",
            "failed to apply window size: {}",
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
            schedule_save_from_physical(window, *size, None, save_token.clone());
        }
        tauri::WindowEvent::ScaleFactorChanged {
            scale_factor,
            new_inner_size,
            ..
        } => {
            schedule_save_from_physical(
                window,
                *new_inner_size,
                Some(*scale_factor),
                save_token.clone(),
            );
        }
        tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed => {
            save_token.fetch_add(1, Ordering::SeqCst);
            if let Err(e) = save_current_window_size(window) {
                app_warn!("window", "save_size", "failed to save window size: {}", e);
            }
        }
        _ => {}
    }
}

fn schedule_save_from_physical(
    window: &tauri::Window,
    size: PhysicalSize<u32>,
    scale_factor: Option<f64>,
    save_token: ResizeSaveToken,
) {
    if !should_persist_size(window) {
        return;
    }
    let scale_factor = match scale_factor {
        Some(value) => value,
        None => match window.scale_factor() {
            Ok(value) => value,
            Err(e) => {
                app_warn!("window", "save_size", "failed to read scale factor: {}", e);
                return;
            }
        },
    };
    let Some(state) = MainWindowState::from_physical(size, scale_factor) else {
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
                "save_size",
                "failed to persist window size: {}",
                e
            );
        }
    });
}

fn save_current_window_size(window: &tauri::Window) -> anyhow::Result<()> {
    if !should_persist_size(window) {
        return Ok(());
    }
    let size = window.inner_size()?;
    let scale_factor = window.scale_factor()?;
    if let Some(state) = MainWindowState::from_physical(size, scale_factor) {
        save_state(state)?;
    }
    Ok(())
}

fn should_persist_size(window: &tauri::Window) -> bool {
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
    Ok(MainWindowState::new(state.width, state.height))
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
            })
        );
        assert_eq!(
            MainWindowState::new(100.0, 100.0),
            Some(MainWindowState {
                width: MIN_MAIN_WINDOW_WIDTH,
                height: MIN_MAIN_WINDOW_HEIGHT,
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
            MainWindowState::from_physical(PhysicalSize::new(2720, 1720), 2.0),
            Some(MainWindowState {
                width: 1360.0,
                height: 860.0,
            })
        );
        assert_eq!(
            MainWindowState::from_physical(PhysicalSize::new(1, 1), 0.0),
            None
        );
    }
}
