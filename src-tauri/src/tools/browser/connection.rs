use anyhow::Result;
use serde_json::Value;

use super::{get_bool, get_str};
use crate::browser_state::get_browser_state;

pub(super) async fn action_connect(args: &Value) -> Result<String> {
    let url = get_str(args, "url").unwrap_or("http://127.0.0.1:9222");

    let mut state = get_browser_state().lock().await;
    if state.is_connected() {
        state.disconnect().await;
    }

    state.connect(url).await?;

    let page_count = state.pages.len();
    let active = state.active_page_id.clone().unwrap_or_default();

    Ok(format!(
        "Connected to Chrome at {}. Found {} page(s). Active page: {}",
        url, page_count, active
    ))
}

pub(super) async fn action_launch(args: &Value) -> Result<String> {
    let executable = get_str(args, "executable_path");
    let headless = get_bool(args, "headless").unwrap_or(false);
    let profile = get_str(args, "profile");

    let mut state = get_browser_state().lock().await;
    if state.is_connected() {
        state.disconnect().await;
    }

    state.launch(executable, headless, profile).await?;

    let page_count = state.pages.len();
    let profile_info = profile
        .map(|p| format!(", profile: {}", p))
        .unwrap_or_default();

    Ok(format!(
        "Chrome launched successfully{}{}. {} page(s) available.",
        if headless { " (headless)" } else { "" },
        profile_info,
        page_count
    ))
}

pub(super) async fn action_disconnect() -> Result<String> {
    let mut state = get_browser_state().lock().await;
    if !state.is_connected() {
        return Ok("Not connected to any browser.".to_string());
    }
    state.disconnect().await;
    Ok("Browser disconnected.".to_string())
}
