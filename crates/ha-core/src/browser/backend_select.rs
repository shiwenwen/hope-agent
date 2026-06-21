//! Backend acquisition.
//!
//! Backend selection.
//!
//! Product policy is "Chrome Extension first, CDP fallback". CDP fallback is
//! allowed only when the caller's action does not require the user's real
//! Chrome tabs or logged-in session state.
//!
//! - [`acquire_backend`] is the entry point used by 8-action handlers.
//! - [`reset_backend`] clears the cache on `profile.disconnect` /
//!   `profile.launch`.
//! - [`peek_active`] is a read-only inspector used by `status` / UI.
//!
//! The active backend is stored as `Arc<dyn BrowserBackend>` so handlers can
//! grab a cheap clone and release the lock before doing IO.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use tokio::sync::Mutex;

use super::backend::BrowserBackend;
use super::cdp_backend::CdpBackend;
use super::extension::{
    current_status, events::emit_extension_required, BrowserBackendContext,
    BrowserBackendRequirement, BrowserExtensionBroker, ExtensionBackend,
};
use super::BrowserBackendPreference;

/// Currently-active backend, if any. Cleared on profile disconnect / launch.
static ACTIVE_BACKEND: Mutex<Option<Arc<dyn BrowserBackend>>> = Mutex::const_new(None);

/// Acquire a backend, creating one if none is active.
///
/// Cached-but-dead backends are detected via [`BrowserBackend::is_alive`] and
/// rebuilt — the alternative would be permanent breakage until the user calls
/// `profile.op=disconnect`. CDP backend is stateless so `is_alive()` is
/// effectively always true; the cache reuse path returns the same `Arc`.
pub async fn acquire_backend() -> Result<Arc<dyn BrowserBackend>> {
    acquire_backend_for(
        BrowserBackendContext::default(),
        BrowserBackendRequirement::ExtensionPreferred,
    )
    .await
}

/// Acquire a backend for a concrete browser action.
///
/// Use [`BrowserBackendRequirement::ExtensionRequired`] for operations that
/// semantically depend on the user's real Chrome state (`open_user_tabs`,
/// `claim`, claimed-tab actions). In that mode CDP fallback is forbidden,
/// because succeeding in an isolated profile would be worse than failing.
pub async fn acquire_backend_for(
    ctx: BrowserBackendContext,
    requirement: BrowserBackendRequirement,
) -> Result<Arc<dyn BrowserBackend>> {
    {
        let mut guard = ACTIVE_BACKEND.lock().await;
        if let Some(b) = guard.as_ref() {
            if b.is_alive().await && cached_backend_satisfies_requirement(&**b, requirement) {
                if requirement == BrowserBackendRequirement::ExtensionPreferred
                    && b.backend_name() == "cdp"
                    && current_status().backend_available
                {
                    app_info!(
                        "browser",
                        "backend_select",
                        "Chrome Extension became available; dropping cached CDP backend"
                    );
                    *guard = None;
                    super::observe_buffer::clear_all();
                    super::cdp_backend::clear_subscribed_pages();
                } else {
                    return Ok(b.clone());
                }
            } else {
                app_warn!(
                    "browser",
                    "backend_select",
                    "Cached {} backend cannot satisfy {:?} or reports dead; dropping and rebuilding",
                    b.backend_name(),
                    requirement
                );
                *guard = None;
                super::observe_buffer::clear_all();
                super::cdp_backend::clear_subscribed_pages();
            }
        }
        if let Some(b) = guard.as_ref() {
            if cached_backend_satisfies_requirement(&**b, requirement) {
                return Ok(b.clone());
            }
            *guard = None;
        }
    }

    let cfg = crate::config::cached_config();
    let browser_cfg = cfg.browser.as_ref();
    let preference = browser_cfg
        .and_then(|b| b.backend_preference)
        .unwrap_or_default();
    let extension_enabled = browser_cfg
        .and_then(|b| b.extension.as_ref())
        .is_none_or(|ext| ext.enabled());

    if preference == BrowserBackendPreference::CdpOnly {
        if requirement == BrowserBackendRequirement::ExtensionRequired {
            let reason = "browser backend is configured as cdp_only";
            let status = current_status();
            emit_extension_required(&ctx, requirement, reason, &status);
            return Err(extension_required_error(reason));
        }
        return cache_backend(Arc::new(CdpBackend::new())).await;
    }

    if extension_enabled {
        let extension_status = current_status();
        let mut fallback_reason = extension_status.message.clone();
        if extension_status.backend_available {
            if let Some(broker) = BrowserExtensionBroker::global() {
                return Ok(Arc::new(ExtensionBackend::new(broker, ctx)));
            }
            fallback_reason =
                "Extension status is ready but broker global is unavailable".to_string();
            app_warn!(
                "browser",
                "backend_select",
                "Extension status is ready but broker global is unavailable; falling back according to requirement"
            );
        } else {
            app_info!(
                "browser",
                "backend_select",
                "Chrome Extension backend unavailable: {}",
                extension_status.message
            );
        }

        if requirement == BrowserBackendRequirement::ExtensionRequired
            || (preference == BrowserBackendPreference::ExtensionOnly
                && requirement != BrowserBackendRequirement::CdpAllowed)
        {
            emit_extension_required(&ctx, requirement, &fallback_reason, &extension_status);
            return Err(extension_required_error(&fallback_reason));
        }
    } else if requirement == BrowserBackendRequirement::ExtensionRequired {
        let reason = "Chrome Extension backend is disabled";
        let status = current_status();
        emit_extension_required(&ctx, requirement, reason, &status);
        return Err(extension_required_error(reason));
    }

    cache_backend(Arc::new(CdpBackend::new())).await
}

async fn cache_backend(backend: Arc<dyn BrowserBackend>) -> Result<Arc<dyn BrowserBackend>> {
    let mut guard = ACTIVE_BACKEND.lock().await;
    *guard = Some(backend.clone());
    Ok(backend)
}

fn cached_backend_satisfies_requirement(
    backend: &dyn BrowserBackend,
    requirement: BrowserBackendRequirement,
) -> bool {
    match requirement {
        BrowserBackendRequirement::ExtensionRequired => backend.backend_name() == "extension",
        BrowserBackendRequirement::ExtensionPreferred | BrowserBackendRequirement::CdpAllowed => {
            true
        }
    }
}

fn extension_required_error(reason: &str) -> anyhow::Error {
    anyhow!(
        "This browser action requires the Hope Agent Chrome Extension. {}. \
         Install or enable the extension to use real Chrome tabs and logged-in sessions, \
         or explicitly choose an isolated CDP browser instead.",
        reason
    )
}

/// Tear down the active backend (e.g. on `profile.disconnect`).
///
/// Future `acquire_backend` calls will reinitialise.
pub async fn reset_backend() {
    let mut guard = ACTIVE_BACKEND.lock().await;
    *guard = None;
    super::observe_buffer::clear_all();
    super::cdp_backend::clear_subscribed_pages();
}

/// Read-only peek at the currently-active backend without acquiring.
pub async fn peek_active() -> Option<Arc<dyn BrowserBackend>> {
    ACTIVE_BACKEND.lock().await.clone()
}

/// Backend for read-only status inspection (e.g. the `status` action).
///
/// Unlike [`peek_active`], this does not depend on a prior tool call having
/// cached a backend. The per-session `ExtensionBackend` carries a ctx and is
/// deliberately never cached — caching it would leak one session's tab scope
/// into another — so when the extension is the effective backend `peek_active`
/// returns `None` and callers would wrongly report "disconnected". Build a
/// session-less probe for global inspection: it lists all real Chrome tabs
/// without disturbing any cached per-session backend, and is cheap (no Chrome
/// process is launched, unlike the CDP backend). Falls back to whatever is
/// cached (CDP) when the extension is not the effective backend.
pub async fn status_backend() -> Option<Arc<dyn BrowserBackend>> {
    let cfg = crate::config::cached_config();
    let browser_cfg = cfg.browser.as_ref();
    let preference = browser_cfg
        .and_then(|b| b.backend_preference)
        .unwrap_or_default();
    if preference != BrowserBackendPreference::CdpOnly {
        let extension_enabled = browser_cfg
            .and_then(|b| b.extension.as_ref())
            .is_none_or(|ext| ext.enabled());
        if extension_enabled && current_status().backend_available {
            if let Some(broker) = BrowserExtensionBroker::global() {
                return Some(Arc::new(ExtensionBackend::new(
                    broker,
                    BrowserBackendContext::default(),
                )));
            }
        }
    }
    peek_active().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn acquire_backend_returns_cdp_backend() {
        reset_backend().await;
        let b = acquire_backend().await.expect("acquire backend");
        assert_eq!(b.backend_name(), "cdp");
        // Re-acquire reuses the cached Arc.
        let b2 = acquire_backend().await.expect("acquire backend again");
        assert!(Arc::ptr_eq(&b, &b2));
        reset_backend().await;
    }

    #[tokio::test]
    async fn extension_required_does_not_fallback_to_cdp() {
        reset_backend().await;
        let res = acquire_backend_for(
            BrowserBackendContext::default(),
            BrowserBackendRequirement::ExtensionRequired,
        )
        .await;
        let msg = match res {
            Ok(_) => panic!("ExtensionRequired must not fall back to CDP"),
            Err(err) => err.to_string(),
        };
        assert!(msg.contains("requires the Hope Agent Chrome Extension"));
        reset_backend().await;
    }
}
