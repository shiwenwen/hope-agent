//! Main-window visibility belongs to the desktop shell. On macOS, `hide`
//! must wait for AppKit's full-screen transition to finish, not a timer or
//! an early `is_fullscreen() == false` snapshot.
use tauri::Manager;

#[cfg(any(target_os = "macos", test))]
mod state;

pub fn initialize(app: &tauri::AppHandle) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    native::initialize(app)?;
    #[cfg(not(target_os = "macos"))]
    let _ = app;
    Ok(())
}

pub fn hide_main_window(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let handle = app.clone();
        if let Err(error) = app.run_on_main_thread(move || native::request_hide(&handle)) {
            ha_core::app_warn!("window", "main:hide", "Cannot schedule hide: {error}");
        }
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(window) = app.get_webview_window("main") {
        if let Err(error) = window.hide() {
            ha_core::app_warn!("window", "main:hide", "Cannot hide main window: {error}");
        }
    }
}

pub fn show_main_window(app: &tauri::AppHandle) {
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        #[cfg(target_os = "macos")]
        native::cancel_pending_hide();
        if let Some(window) = handle.get_webview_window("main") {
            let result = window
                .show()
                .and_then(|()| window.unminimize())
                .and_then(|()| window.set_focus());
            match result {
                Ok(()) => ha_core::app_info!("window", "main:show", "Main window shown"),
                Err(error) => {
                    ha_core::app_warn!("window", "main:show", "Cannot show main window: {error}")
                }
            }
        }
    }) {
        ha_core::app_warn!("window", "main:show", "Cannot schedule show: {error}");
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::state::{Action, Phase, VisibilityState};
    use block2::{DynBlock, RcBlock};
    use objc2::rc::Retained;
    use objc2_app_kit::{
        NSWindow, NSWindowDidEnterFullScreenNotification, NSWindowDidExitFullScreenNotification,
        NSWindowWillEnterFullScreenNotification, NSWindowWillExitFullScreenNotification,
    };
    use objc2_foundation::{NSNotification, NSNotificationCenter};
    use std::{
        ptr::NonNull,
        sync::{Mutex, OnceLock},
        time::Duration,
    };
    use tauri::Manager;

    static STATE: OnceLock<Mutex<VisibilityState>> = OnceLock::new();

    // Called by app_setup on the AppKit main thread, once for the process's
    // resident main window. Observers filter the exact native window object.
    pub(super) fn initialize(app: &tauri::AppHandle) -> tauri::Result<()> {
        let Some(window) = app.get_webview_window("main") else {
            return Ok(());
        };
        let initial_phase = if window.is_fullscreen()? {
            Phase::Fullscreen
        } else {
            Phase::Windowed
        };
        let mut state = VisibilityState::default();
        state.observe(initial_phase);
        if STATE.set(Mutex::new(state)).is_err() {
            return Ok(());
        }
        let ns_window: &NSWindow = unsafe { &*window.ns_window()?.cast() };
        let center = NSNotificationCenter::defaultCenter();
        // The main window is never destroyed by Close/Cmd+Q. These four
        // process-lifetime observers retain no raw NSWindow pointer in callbacks.
        for (name, phase) in unsafe {
            [
                (NSWindowWillEnterFullScreenNotification, Phase::Entering),
                (NSWindowDidEnterFullScreenNotification, Phase::Fullscreen),
                (NSWindowWillExitFullScreenNotification, Phase::Exiting),
                (NSWindowDidExitFullScreenNotification, Phase::Windowed),
            ]
        } {
            let handle = app.clone();
            let callback = RcBlock::new(move |_: NonNull<NSNotification>| {
                STATE
                    .get()
                    .expect("visibility initialized")
                    .lock()
                    .expect("visibility lock")
                    .observe(phase);
                ha_core::app_info!(
                    "window",
                    "main:fullscreen",
                    "Main window transition: {phase:?}"
                );
                // Tauri runs main-thread dispatch inline when already on main.
                // Hop through its async runtime to leave AppKit's notification
                // callback before requesting another transition or hiding.
                schedule_drive(handle.clone());
            });
            let callback: &DynBlock<dyn Fn(NonNull<NSNotification>)> = &callback;
            let observer = unsafe {
                center.addObserverForName_object_queue_usingBlock(
                    Some(name),
                    Some(ns_window),
                    None,
                    callback,
                )
            };
            let _ = Retained::into_raw(observer);
        }
        Ok(())
    }

    pub(super) fn cancel_pending_hide() {
        if let Some(state) = STATE.get() {
            state.lock().expect("visibility lock").cancel();
        }
    }

    pub(super) fn request_hide(app: &tauri::AppHandle) {
        let Some(state) = STATE.get() else {
            ha_core::app_warn!(
                "window",
                "main:hide",
                "Visibility observer unavailable; leaving window visible"
            );
            return;
        };
        let Some(window) = app.get_webview_window("main") else {
            return;
        };
        let fullscreen = match window.is_fullscreen() {
            Ok(value) => value,
            Err(error) => {
                ha_core::app_warn!(
                    "window",
                    "main:hide",
                    "Cannot read fullscreen state; leaving window visible: {error}"
                );
                return;
            }
        };
        let Some(ticket) = state
            .lock()
            .expect("visibility lock")
            .request_hide(fullscreen)
        else {
            return;
        };
        ha_core::app_info!(
            "window",
            "main:hide",
            "Hide requested (fullscreen={fullscreen}, ticket={ticket})"
        );
        drive(app);
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            let _ = handle.run_on_main_thread(move || {
                if STATE
                    .get()
                    .expect("visibility initialized")
                    .lock()
                    .expect("visibility lock")
                    .expire(ticket)
                {
                    ha_core::app_warn!(
                        "window",
                        "main:hide",
                        "Fullscreen exit timed out; leaving window visible (ticket={ticket})"
                    );
                }
            });
        });
    }

    fn schedule_drive(app: tauri::AppHandle) {
        tauri::async_runtime::spawn(async move {
            let handle = app.clone();
            if let Err(error) = app.run_on_main_thread(move || drive(&handle)) {
                ha_core::app_warn!(
                    "window",
                    "main:hide",
                    "Cannot resume deferred hide: {error}"
                );
            }
        });
    }

    fn drive(app: &tauri::AppHandle) {
        let action = STATE
            .get()
            .expect("visibility initialized")
            .lock()
            .expect("visibility lock")
            .take_action();
        let Some(action) = action else {
            return;
        };
        let Some(window) = app.get_webview_window("main") else {
            cancel_pending_hide();
            return;
        };
        let result = match action {
            Action::Hide => window.hide(),
            Action::ExitFullscreen => window.set_fullscreen(false),
        };
        match result {
            Ok(()) => ha_core::app_info!("window", "main:hide", "Main window action: {action:?}"),
            Err(error) => {
                STATE
                    .get()
                    .expect("visibility initialized")
                    .lock()
                    .expect("visibility lock")
                    .abort();
                ha_core::app_warn!(
                    "window",
                    "main:hide",
                    "Main window action {action:?} failed; leaving window visible: {error}"
                );
            }
        }
    }
}
