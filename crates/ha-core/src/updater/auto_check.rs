//! Headless background auto-update loop (primary-gated).
//!
//! The desktop shell already runs its own periodic check through
//! `@tauri-apps/plugin-updater` (see `src/lib/desktopUpdater.ts`), so this loop
//! **only runs in non-desktop formfactors** (`hope-agent server` / ACP) to give
//! them the parity the GUI has had: a periodic check that surfaces new releases
//! and — when `auto_download` is on — silently pre-stages the verified build so
//! the eventual install is a no-network swap.
//!
//! Mirrors the dreaming cron loop ([`crate::memory::dreaming::cron_loop`]):
//! spawned once at startup behind `runtime_lock::is_primary`, re-reads
//! `cached_config().auto_update` on every wake, and reschedules itself when the
//! user edits the config (it wakes on `config:changed`).
//!
//! It never swaps the binary itself — that always goes through the
//! user-confirmed `app_update install` tool. This loop is check + (optional)
//! pre-download only.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::json;
use tokio::sync::Notify;

use super::config::AutoUpdateConfig;
use super::RecommendedPath;

/// Delay before the first check so it doesn't contend with the cold-start burst
/// (DB open, embedding init, channel auto-start, …).
const INITIAL_DELAY_SECS: u64 = 60;

#[derive(Default)]
struct AutoCheckState {
    /// Last version we emitted an `app_update:available` event for — dedups the
    /// notification so a daily check doesn't re-fire the same banner forever.
    last_notified: Option<String>,
    /// Last version we successfully pre-staged — avoids re-downloading an
    /// already-staged build on every interval.
    last_staged: Option<String>,
}

fn state() -> &'static Mutex<AutoCheckState> {
    static S: OnceLock<Mutex<AutoCheckState>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(AutoCheckState::default()))
}

/// Spawn the periodic auto-update loop. No-op on desktop (the JS updater owns
/// that path) and intended to be called only when this process is the runtime
/// primary.
pub fn spawn_auto_update_loop() {
    if crate::app_init::is_desktop() {
        return;
    }

    let notify = Arc::new(Notify::new());
    if let Some(bus) = crate::get_event_bus() {
        let mut rx = bus.subscribe();
        let notify_for_sub = notify.clone();
        // Wake on any config:changed — `mutate_config` emits `{category:"app"}`
        // while ha-settings emits `"auto_update"`, so an `auto_update`-only
        // filter would miss GUI saves.
        tokio::spawn(async move {
            while let Ok(evt) = rx.recv().await {
                if evt.name == "config:changed" {
                    notify_for_sub.notify_one();
                }
            }
        });
    } else {
        app_warn!(
            "self_update",
            "auto_check",
            "EventBus not initialized — auto-update loop won't respond to live config edits"
        );
    }

    tokio::spawn(async move {
        // Startup hygiene: clear stale half-downloads left by prior crashes.
        super::staging::prune(None);

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(INITIAL_DELAY_SECS)) => {}
            _ = notify.notified() => {}
        }

        loop {
            let cfg = crate::config::cached_config().auto_update.clone();
            if !cfg.check_enabled {
                // Disabled: sleep until the config changes.
                notify.notified().await;
                continue;
            }

            run_check_once(&cfg).await;

            let wait = Duration::from_secs(cfg.clamped_interval_secs());
            app_info!(
                "self_update",
                "auto_check",
                "next auto-update check in {:.1}h",
                cfg.clamped_interval_hours()
            );
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = notify.notified() => {}
            }
        }
    });
}

async fn run_check_once(cfg: &AutoUpdateConfig) {
    let (outcome, manifest) = match super::check_update_full().await {
        Ok(v) => v,
        Err(e) => {
            app_warn!("self_update", "auto_check", "update check failed: {}", e);
            return;
        }
    };
    if !outcome.has_update {
        app_debug!(
            "self_update",
            "auto_check",
            "up to date (current {})",
            outcome.current_version
        );
        return;
    }

    let version = outcome.latest_version.clone();
    app_info!(
        "self_update",
        "auto_check",
        "update available: {} → {} (path {:?})",
        outcome.current_version,
        version,
        outcome.recommended_path
    );

    if cfg.notify {
        let fresh = {
            let mut g = state().lock().unwrap_or_else(|p| p.into_inner());
            if g.last_notified.as_deref() == Some(version.as_str()) {
                false
            } else {
                g.last_notified = Some(version.clone());
                true
            }
        };
        if fresh {
            if let Some(bus) = crate::get_event_bus() {
                bus.emit(
                    "app_update:available",
                    json!({
                        "currentVersion": outcome.current_version,
                        "version": version,
                        "notes": outcome.notes,
                        "pubDate": outcome.pub_date,
                        "recommendedPath": outcome.recommended_path,
                    }),
                );
            }
        }
    }

    // Silent pre-download: only the self-contained route stages a bare binary.
    // PackageManager / Tauri / ManualPrompt have nothing to pre-fetch here.
    if cfg.auto_download && outcome.recommended_path == RecommendedPath::SelfContained {
        let already = {
            let g = state().lock().unwrap_or_else(|p| p.into_inner());
            g.last_staged.as_deref() == Some(version.as_str())
        };
        if already {
            return;
        }
        let job_id = format!("autostage_{}", uuid::Uuid::new_v4().simple());
        match super::self_contained::stage_only(&job_id, Some(&version), Some(manifest)).await {
            Ok(staged_version) => {
                {
                    let mut g = state().lock().unwrap_or_else(|p| p.into_inner());
                    g.last_staged = Some(staged_version.clone());
                }
                app_info!(
                    "self_update",
                    "auto_check",
                    "pre-staged {} for instant install",
                    staged_version
                );
                if let Some(bus) = crate::get_event_bus() {
                    bus.emit("app_update:staged", json!({ "version": staged_version }));
                }
            }
            Err(e) => app_warn!(
                "self_update",
                "auto_check",
                "silent pre-download of {} failed: {}",
                version,
                e
            ),
        }
    }
}
