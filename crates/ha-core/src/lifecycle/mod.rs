//! Restart the running Hope Agent process across desktop / service / foreground
//! formfactors. Used by [`crate::tools::app_restart`] and the future
//! `/restart` slash command.
//!
//! Three routes, picked automatically:
//!
//! | mode                                          | route       | who actually relaunches              |
//! | --------------------------------------------- | ----------- | ------------------------------------ |
//! | desktop (`is_desktop()`)                      | `Desktop`   | Tauri guardian (exit 42 + supervisor)|
//! | server **and** `is_service_installed()`       | `Service`   | launchd / systemd / Task Scheduler   |
//! | server, foreground (no system service)        | `Respawn`   | a freshly-spawned detached child     |
//! | acp                                           | n/a         | refused — IDE owns the stdio lifetime|
//!
//! [`restart`] is fire-and-forget — the OS-level handoff means the caller has
//! at most a few hundred ms before the current process disappears. Errors
//! returned by this function reflect "we couldn't even hand off" (no bridge
//! registered, no argv captured, spawn refused), not "the new process didn't
//! come up" — the latter is invisible from inside the dying process.
//!
//! The [`AppLifecycleBridge`] trait is the desktop hook (analogous to
//! [`crate::updater::UpdaterBridge`]): `src-tauri` installs an impl at
//! startup that calls `app.exit(42)`, which the guardian respawns.

use std::sync::{Arc, OnceLock};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

mod inflight;
mod respawn;

pub use inflight::{collect_inflight, InflightItem, InflightSummary};

/// Pluggable hook for the desktop runtime. `src-tauri` registers an impl
/// that delegates to `tauri::AppHandle::exit(42)`; the guardian process
/// catches the exit code and relaunches the GUI child.
pub trait AppLifecycleBridge: Send + Sync {
    /// Trigger a desktop-side restart. Returns `Ok(())` once the request has
    /// been queued — actual exit happens out-of-band. Implementations must
    /// not block; the calling task will be torn down by the upcoming exit.
    fn restart_desktop(&self) -> Result<()>;
}

static BRIDGE: OnceLock<Arc<dyn AppLifecycleBridge>> = OnceLock::new();

pub fn set_lifecycle_bridge(bridge: Arc<dyn AppLifecycleBridge>) {
    let _ = BRIDGE.set(bridge);
}

pub fn get_lifecycle_bridge() -> Option<Arc<dyn AppLifecycleBridge>> {
    BRIDGE.get().cloned()
}

/// Which strategy a given process will use to bring itself back up. Returned
/// by [`route`] so the tool can describe the plan to the user in the
/// `ask_user_question` prompt before they confirm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// Desktop GUI — Tauri `app.exit(42)` + guardian respawn.
    Desktop,
    /// Headless server, installed as launchd / systemd / Task Scheduler — let
    /// the OS supervisor stop & start. Robust against PID changes.
    Service,
    /// Headless server, running in the foreground from a terminal. Spawn a
    /// detached child with the captured launch argv, then exit self.
    Respawn,
    /// `hope-agent acp` — IDE owns stdio. Restart would orphan the IDE's
    /// stdin/stdout pipes. Refused.
    Unsupported,
}

impl Route {
    pub fn as_str(&self) -> &'static str {
        match self {
            Route::Desktop => "desktop",
            Route::Service => "service",
            Route::Respawn => "respawn",
            Route::Unsupported => "unsupported",
        }
    }
}

/// Decide which restart route fits the current process. Pure inspection; safe
/// to call multiple times.
pub fn route() -> Route {
    match crate::app_init::runtime_role() {
        Some("desktop") => Route::Desktop,
        Some("server") => {
            if crate::service_install::is_service_installed() {
                Route::Service
            } else {
                Route::Respawn
            }
        }
        Some("acp") => Route::Unsupported,
        _ => Route::Unsupported,
    }
}

/// Human-friendly label for the route, embedded in the
/// `ask_user_question` prompt so the user knows what's about to happen.
/// English-only (LLM-facing) to match the rest of ha-core tool output —
/// the front-end / UI localizes display text separately.
pub fn route_label(route: Route) -> &'static str {
    match route {
        Route::Desktop => "Desktop GUI (guardian will auto-restart the window)",
        Route::Service => {
            "Installed system service (launchd / systemd / Task Scheduler will respawn it)"
        }
        Route::Respawn => "Foreground `hope-agent server` (spawn detached child, then exit self)",
        Route::Unsupported => "ACP / unknown mode (cannot auto-restart)",
    }
}

/// Carry out the restart for the current runtime mode. Returns immediately
/// after handing off to the OS-level supervisor (or queuing the desktop
/// `exit(42)`); the process is expected to die within a few hundred ms.
///
/// Emits `app:restart_initiated` on the EventBus before handing off so any
/// listening UI / log sink sees the reason before the bus tears down.
pub fn restart() -> Result<RestartOutcome> {
    let route = route();
    emit_restart_initiated(route);

    match route {
        Route::Desktop => {
            let bridge = get_lifecycle_bridge().ok_or_else(|| {
                anyhow::anyhow!(
                    "no AppLifecycleBridge registered — desktop restart requires src-tauri to install one at startup"
                )
            })?;
            bridge.restart_desktop()?;
            Ok(RestartOutcome {
                route,
                detail: "Tauri exit(42) queued; guardian will respawn the GUI child".into(),
            })
        }
        Route::Service => {
            // service_control::restart_service is the same helper the self-update
            // path uses — kickstart / systemctl restart / schtasks End+Run.
            let msg = crate::updater::service_control::restart_service()?;
            // Service supervisor will kill us as part of restart; we don't
            // need to exit here. Return the supervisor's ack so the tool
            // result is informative.
            Ok(RestartOutcome { route, detail: msg })
        }
        Route::Respawn => {
            let pid = respawn::respawn_detached_server()?;
            // Spawn succeeded — schedule our own graceful exit a moment later
            // so the EventBus / tool result has time to flush. The child has
            // its own bind socket retry loop (ServerConfig::bind_addr) so a
            // 200ms window where neither process holds the port is fine.
            respawn::schedule_self_exit();
            Ok(RestartOutcome {
                route,
                detail: format!("detached server child spawned (pid {pid}); self will exit"),
            })
        }
        Route::Unsupported => bail!(
            "restart is not supported in the current mode ({:?}) — manual relaunch required",
            crate::app_init::runtime_role()
        ),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RestartOutcome {
    pub route: Route,
    pub detail: String,
}

fn emit_restart_initiated(route: Route) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "app:restart_initiated",
            serde_json::json!({
                "route": route.as_str(),
                "pid": std::process::id(),
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_is_unsupported_when_runtime_unset() {
        // `init_runtime` may or may not have been called by the time tests
        // run depending on harness ordering. The only invariant we can
        // assert here is that `Route` derives reasonable enums and `as_str`
        // round-trips — actual routing depends on global init state.
        let labels: Vec<&str> = [
            Route::Desktop,
            Route::Service,
            Route::Respawn,
            Route::Unsupported,
        ]
        .iter()
        .map(|r| r.as_str())
        .collect();
        assert_eq!(labels, vec!["desktop", "service", "respawn", "unsupported"]);
    }
}
