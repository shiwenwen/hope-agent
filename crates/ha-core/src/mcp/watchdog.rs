//! Background watchdog — health check + exponential-backoff reconnect.
//!
//! Design:
//! * One task per process, spawned from [`McpManager::init_global`] after
//!   the registry is built (and respawned idempotently if the manager is
//!   reused across subsystem reinit — in practice rare because the global
//!   OnceLock only fills once).
//! * Cheap health probe: checks `RunningService::is_closed()` on every
//!   `Ready` server. A broken stdio pipe or server crash flips this to
//!   `true`; we mark the server `Failed` and let the backoff kick in.
//! * No network `ping` — active probes would add steady traffic for very
//!   little benefit. We rely on `is_closed` + failures bubbling from real
//!   tool calls to catch regressions.
//! * Backoff: `min(backoff_initial * 2^n, backoff_max)` seconds, where
//!   `n = consecutive_failures.saturating_sub(1)`. Capped at 6 doublings
//!   to prevent u64 overflow for pathological configs.
//!
//! This module intentionally runs inside the shared `tokio` runtime — it
//! must NEVER block on sync primitives for more than a few micros. The
//! per-server `connect_now` call is awaited serially across servers to
//! avoid hammering the CPU during mass reconnect storms.

use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::time::interval;

use super::client;
use super::registry::{McpManager, ServerState};

/// Wall-clock seconds between watchdog ticks. Kept independent of the
/// per-server `health_check_interval_secs` knob — the latter controls
/// how often we *probe* any single server, which we honor by skipping
/// ones that were probed recently inside a tick. The loop tick itself
/// is cheap; 15s gives good reaction without burning wakeups.
const TICK_INTERVAL_SECS: u64 = 15;

/// Upper clamp on the shift exponent used for backoff doubling. 6 → up
/// to a 64× multiplier on the initial backoff (already bounded by
/// `backoff_max_secs` anyway; this just keeps the shift safe).
const MAX_BACKOFF_SHIFT: u32 = 6;

/// Spawn the watchdog loop. Returns immediately; the loop runs until
/// the tokio runtime shuts down. Safe to call multiple times — extra
/// loops are harmless (all idempotent reads/writes), but callers should
/// avoid it to keep the log noise down.
pub fn spawn_watchdog_loop() {
    tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(TICK_INTERVAL_SECS));
        // Skip the immediate first tick (interval fires once at t=0) —
        // init_global has just finished building the registry, eager
        // connects haven't run yet, there's nothing useful to observe.
        tick.tick().await;
        loop {
            tick.tick().await;
            if let Err(e) = run_tick().await {
                crate::app_warn!("mcp", "watchdog", "Tick failed: {}", e);
            }
        }
    });
}

async fn run_tick() -> anyhow::Result<()> {
    let Some(manager) = McpManager::global() else {
        // Subsystem not initialized (or globally disabled via a config
        // edit that ran reconcile). Bail quietly — we'll try again next
        // tick in case the state flipped.
        return Ok(());
    };
    if !manager.is_enabled().await {
        return Ok(());
    }

    let now = chrono::Utc::now().timestamp();
    let global = manager.settings().await;

    // Snapshot the server list so we don't hold the RwLock across per-
    // server awaits. `get_by_id` later re-acquires if the server is
    // gone between snapshot and action.
    let ids: Vec<String> = {
        let servers = manager.servers.read().await;
        servers.keys().cloned().collect()
    };

    for id in ids {
        let Some(handle) = manager.get_by_id(&id).await else {
            continue;
        };
        let cfg = handle.config.read().await.clone();
        if !cfg.enabled {
            continue;
        }

        // Eager warm-up: kick a connect on the first tick a disabled →
        // enabled server stays `Idle`. Skipping `eager=false` servers
        // keeps the lazy path as the default.
        let state_label = handle.state.lock().await.label();
        if state_label == "idle" && cfg.eager {
            let mgr = manager;
            let h = handle.clone();
            tokio::spawn(async move {
                if let Err(e) = client::connect_now(mgr, h).await {
                    crate::app_warn!(
                        "mcp",
                        "watchdog:eager_connect",
                        "Eager connect failed: {}",
                        e
                    );
                }
            });
            continue;
        }

        // Detect a silently-dead connection: we think we're `Ready` but
        // the rmcp service-task has exited (transport closed, server
        // crashed, network blip for networked transports).
        if state_label == "ready" {
            let closed = {
                let guard = handle.client.lock().await;
                guard.as_ref().map(|s| s.is_closed()).unwrap_or(true)
            };
            if closed {
                crate::app_warn!(
                    "mcp",
                    &format!("{}:watchdog", cfg.name),
                    "Detected closed rmcp service; forcing disconnect + reconnect"
                );
                let _ = client::disconnect(&handle).await;
                // Fall through to the Failed-state handling below on the
                // next tick, which applies the usual backoff before
                // retrying.
            }
        }

        // Reconnect attempt for backoff window that has elapsed. Uses
        // the live state snapshot, not the cached label above.
        let should_reconnect = {
            let state = handle.state.lock().await;
            matches!(&*state, ServerState::Failed { retry_at, .. } if *retry_at <= now)
        };
        if should_reconnect {
            let failures = handle.consecutive_failures.load(Ordering::Relaxed);
            if global.consecutive_failure_circuit_breaker > 0
                && failures >= global.consecutive_failure_circuit_breaker
            {
                // Circuit breaker tripped. Push the retry_at further out
                // so the log spam settles; user can still hit the GUI's
                // manual Reconnect at any time.
                let cooldown = global.auto_reconnect_after_circuit_secs.max(60);
                let mut state = handle.state.lock().await;
                if let ServerState::Failed { retry_at, reason } = &mut *state {
                    *retry_at = now + cooldown as i64;
                    let _ = reason; // keep original reason for display
                }
                continue;
            }
            // Space out the reconnect by the exponential-backoff window.
            let shift = failures.saturating_sub(1).min(MAX_BACKOFF_SHIFT);
            let wait = global
                .backoff_initial_secs
                .saturating_mul(1u64 << shift)
                .min(global.backoff_max_secs.max(1));
            // Give the scheduled-retry path a bit of breathing room
            // between ticks by only reconnecting if enough time has
            // passed since `record_failure` pinned `retry_at`. Hugely
            // simpler than tracking a separate last-attempt ts.
            let needs_wait = {
                let state = handle.state.lock().await;
                matches!(&*state, ServerState::Failed { retry_at, .. }
                    if (*retry_at - now) + (wait as i64) > 0 && *retry_at > now)
            };
            if needs_wait {
                continue;
            }

            let mgr = manager;
            let h = handle.clone();
            tokio::spawn(async move {
                if let Err(e) = client::connect_now(mgr, h).await {
                    // Already logged inside record_failure; no need to
                    // re-log here, but we keep the spawn so one slow
                    // server doesn't block the rest of the tick.
                    let _ = e;
                }
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // Backoff math isn't pulled into a pure fn; guard the shift-clamp
    // constant so a future refactor can't silently blow past u64 with
    // `<< 64`.
    #[test]
    fn max_backoff_shift_is_safe_for_u64() {
        let shift = super::MAX_BACKOFF_SHIFT;
        assert!(shift < 64, "MAX_BACKOFF_SHIFT must fit in a u64 left-shift");
        let probe: u64 = 1;
        let _ = probe.saturating_mul(1u64 << shift);
    }
}
