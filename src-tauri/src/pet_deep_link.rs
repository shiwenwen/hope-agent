use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use tauri::{Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;

static PENDING_INSTALL_LINK: OnceLock<Mutex<Option<String>>> = OnceLock::new();
const MAX_INSTALL_LINK_LENGTH: usize = 8 * 1024;

fn pending() -> &'static Mutex<Option<String>> {
    PENDING_INSTALL_LINK.get_or_init(|| Mutex::new(None))
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct InstallLinkEvent {
    link: String,
}

fn supported_install_link(url: &url::Url) -> bool {
    url.as_str().len() <= MAX_INSTALL_LINK_LENGTH
        && url.scheme() == "hope-agent"
        && url.host_str() == Some("pets")
        && url.path() == "/install"
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
}

fn receive(app: &tauri::AppHandle, url: &url::Url) {
    if !supported_install_link(url) {
        ha_core::app_warn!(
            "pet",
            "deep_link_rejected",
            "Rejected an unsupported Hope Agent pet deep link"
        );
        return;
    }
    let link = url.as_str().to_owned();
    *pending()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(link.clone());
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    let _ = app.emit("pet:install_link", InstallLinkEvent { link });
}

pub(crate) fn setup(app: &tauri::App) {
    #[cfg(any(target_os = "linux", all(debug_assertions, windows)))]
    if let Err(error) = app.deep_link().register_all() {
        ha_core::app_warn!(
            "pet",
            "deep_link_registration",
            "Could not register Hope Agent pet deep links: {}",
            error
        );
    }

    match app.deep_link().get_current() {
        Ok(Some(urls)) => {
            for url in urls {
                receive(app.handle(), &url);
            }
        }
        Ok(None) => {}
        Err(error) => ha_core::app_warn!(
            "pet",
            "deep_link_startup",
            "Could not inspect startup pet deep links: {}",
            error
        ),
    }
    let handle = app.handle().clone();
    app.deep_link().on_open_url(move |event| {
        for url in event.urls() {
            receive(&handle, &url);
        }
    });
}

pub(crate) fn take_pending() -> Option<String> {
    pending()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_the_install_shape() {
        assert!(supported_install_link(
            &url::Url::parse(
                "hope-agent://pets/install?name=Hope&imageUrl=https%3A%2F%2Fexample.com%2Fpet.png"
            )
            .unwrap()
        ));
        for rejected in [
            "hope-agent://pets/other",
            "hope-agent://other/install",
            "hope-agent://user@pets/install",
            "hope-agent://pets/install#fragment",
            "https://pets/install",
        ] {
            assert!(!supported_install_link(&url::Url::parse(rejected).unwrap()));
        }
        let oversized = format!(
            "hope-agent://pets/install?name=Hope&imageUrl=https%3A%2F%2Fexample.com%2F{}",
            "a".repeat(MAX_INSTALL_LINK_LENGTH)
        );
        assert!(!supported_install_link(
            &url::Url::parse(&oversized).unwrap()
        ));
    }
}
