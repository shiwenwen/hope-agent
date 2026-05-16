//! Backend acquisition.
//!
//! There is only one backend implementation:
//! [`super::cdp_backend::CdpBackend`]. This module keeps the
//! caching layer around so future Playwright / WebDriver implementations
//! could slot in without churning callsites.
//!
//! - [`acquire_backend`] is the entry point used by 8-action handlers.
//! - [`reset_backend`] clears the cache on `profile.disconnect` /
//!   `profile.launch`.
//! - [`peek_active`] is a read-only inspector used by `status` / UI.
//!
//! The active backend is stored as `Arc<dyn BrowserBackend>` so handlers can
//! grab a cheap clone and release the lock before doing IO.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;

use super::backend::BrowserBackend;
use super::cdp_backend::CdpBackend;

/// Currently-active backend, if any. Cleared on profile disconnect / launch.
static ACTIVE_BACKEND: Mutex<Option<Arc<dyn BrowserBackend>>> = Mutex::const_new(None);

/// Acquire a backend, creating one if none is active.
///
/// Cached-but-dead backends are detected via [`BrowserBackend::is_alive`] and
/// rebuilt — the alternative would be permanent breakage until the user calls
/// `profile.op=disconnect`. CDP backend is stateless so `is_alive()` is
/// effectively always true; the cache reuse path returns the same `Arc`.
pub async fn acquire_backend() -> Result<Arc<dyn BrowserBackend>> {
    {
        let mut guard = ACTIVE_BACKEND.lock().await;
        if let Some(b) = guard.as_ref() {
            if b.is_alive().await {
                return Ok(b.clone());
            }
            app_warn!(
                "browser",
                "backend_select",
                "Cached {} backend reports dead; dropping and rebuilding",
                b.backend_name()
            );
            *guard = None;
            super::observe_buffer::clear_all();
            super::cdp_backend::clear_subscribed_pages();
        }
    }

    let backend: Arc<dyn BrowserBackend> = Arc::new(CdpBackend::new());
    let mut guard = ACTIVE_BACKEND.lock().await;
    *guard = Some(backend.clone());
    Ok(backend)
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
}
