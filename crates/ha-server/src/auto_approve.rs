//! Headless-server tool auto-approve switch.
//!
//! `hope-agent server` exposes the chat API over HTTP/WS. Tool approval
//! prompts are emitted as EventBus events and only the desktop / browser UI
//! (or an IM channel listener) knows how to surface them. Plain HTTP clients
//! that don't subscribe to those events end up with every approval waiting
//! out the 5-minute timeout and then being denied — which appears to the
//! caller as "the model can't run any shell command".
//!
//! For container / CI / pipeline deployments where the operator already
//! trusts whatever runs in their tenant, this module exposes a process-
//! scoped switch that flips every `auto_approve_tools` boolean on the HTTP
//! chat entry point to `true`. Set by either:
//!
//! 1. CLI flag `--auto-approve-tools` on `hope-agent server`.
//! 2. Env var `HA_SERVER_AUTO_APPROVE_TOOLS=1` (Docker-friendly).
//!
//! Neither persists to disk. Sets `ChatEngineParams.auto_approve_tools=true`
//! on every HTTP chat — equivalent to an IM account with `auto_approve_tools=true`.
//! Bypasses ALL approval gates (dangerous-commands / protected-paths / edit-command
//! audits included). [`ha_core::security::dangerous`] is a strict superset: it also
//! silences dispatcher-level `app_warn!` audit logs.

use std::sync::atomic::{AtomicBool, Ordering};

static SERVER_AUTO_APPROVE: AtomicBool = AtomicBool::new(false);

/// CLI flag the user passes to `hope-agent server`.
pub const FLAG: &str = "--auto-approve-tools";

/// Env-var equivalent of [`FLAG`]. Anything truthy (`"1"`, `"true"`, `"yes"`,
/// case-insensitive) enables auto-approve mode. Anything else — including
/// an empty value — leaves it off.
pub const ENV_VAR: &str = "HA_SERVER_AUTO_APPROVE_TOOLS";

pub fn set_active(v: bool) {
    SERVER_AUTO_APPROVE.store(v, Ordering::Relaxed);
}

/// Snapshot of the current flag. Read by HTTP route handlers when building
/// `ChatEngineParams.auto_approve_tools`. Returns true when either the CLI
/// flag or the env var is set — caller can't tell which (and shouldn't
/// need to; check `ENV_VAR` / parse argv explicitly if the source matters).
pub fn is_active() -> bool {
    SERVER_AUTO_APPROVE.load(Ordering::Relaxed)
}

/// Parse the env var to a boolean. Truthy: `1` / `true` / `yes` / `on`
/// (case-insensitive). Everything else (including empty / unset) is false.
pub fn env_truthy(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_truthy_matches_common_yes_values() {
        for v in ["1", "true", "TRUE", "yes", "YES", "on", " true "] {
            assert!(env_truthy(v), "expected truthy: {v:?}");
        }
    }

    #[test]
    fn env_truthy_rejects_falsy_and_garbage() {
        for v in ["", " ", "0", "false", "no", "off", "maybe", "1.0"] {
            assert!(!env_truthy(v), "expected falsy: {v:?}");
        }
    }
}
