use anyhow::Result;
use serde_json::Value;

use super::{get_str, require_browser};
use crate::browser_state::get_browser_state;

pub(super) async fn action_navigate(args: &Value) -> Result<String> {
    require_browser().await?;
    let url = get_str(args, "url").ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;

    {
        let ssrf_cfg = &crate::config::cached_config().ssrf;
        crate::security::ssrf::check_url(url, ssrf_cfg.browser(), &ssrf_cfg.trusted_hosts).await?;
    }

    let state = get_browser_state().lock().await;
    let page = state.get_active_page()?;

    page.goto(url)
        .await
        .map_err(|e| anyhow::anyhow!("Navigation failed: {}", e))?;

    // Wait a bit for page load
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let title: String = page
        .evaluate("document.title")
        .await
        .ok()
        .and_then(|r| r.into_value().ok())
        .unwrap_or_else(|| "untitled".to_string());

    let current_url = page
        .url()
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| url.to_string());

    // Clear element refs (page changed)
    drop(state);
    let mut state = get_browser_state().lock().await;
    state.element_refs.clear();
    state.snapshot_url = None;

    Ok(format!("Navigated to: {} - \"{}\"", current_url, title))
}

pub(super) async fn action_go_back() -> Result<String> {
    require_browser().await?;
    let state = get_browser_state().lock().await;
    let page = state.get_active_page()?;

    page.evaluate("history.back()")
        .await
        .map_err(|e| anyhow::anyhow!("Go back failed: {}", e))?;

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let url = page
        .url()
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "unknown".to_string());

    drop(state);
    let mut state = get_browser_state().lock().await;
    state.element_refs.clear();
    state.snapshot_url = None;

    Ok(format!("Navigated back to: {}", url))
}

pub(super) async fn action_go_forward() -> Result<String> {
    require_browser().await?;
    let state = get_browser_state().lock().await;
    let page = state.get_active_page()?;

    page.evaluate("history.forward()")
        .await
        .map_err(|e| anyhow::anyhow!("Go forward failed: {}", e))?;

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let url = page
        .url()
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "unknown".to_string());

    drop(state);
    let mut state = get_browser_state().lock().await;
    state.element_refs.clear();
    state.snapshot_url = None;

    Ok(format!("Navigated forward to: {}", url))
}

pub(super) async fn action_list_pages() -> Result<String> {
    require_browser().await?;
    let mut state = get_browser_state().lock().await;
    state.refresh_pages().await?;

    if state.pages.is_empty() {
        return Ok("No pages open.".to_string());
    }

    let active_id = state.active_page_id.clone().unwrap_or_default();
    let mut lines = vec!["Open pages:".to_string()];

    for (id, page) in &state.pages {
        let url = page
            .url()
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "about:blank".to_string());
        let marker = if *id == active_id { " [active]" } else { "" };
        lines.push(format!("  - {} {}{}", id, url, marker));
    }

    Ok(lines.join("\n"))
}

pub(super) async fn action_new_page(args: &Value) -> Result<String> {
    require_browser().await?;
    let url = get_str(args, "url").unwrap_or("about:blank");

    if url != "about:blank" {
        let ssrf_cfg = &crate::config::cached_config().ssrf;
        crate::security::ssrf::check_url(url, ssrf_cfg.browser(), &ssrf_cfg.trusted_hosts).await?;
    }

    let mut state = get_browser_state().lock().await;
    let browser = state
        .browser
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not connected"))?;

    let page = browser
        .new_page(url)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create new page: {}", e))?;

    let target_id = page.target_id().as_ref().to_string();
    state.active_page_id = Some(target_id.clone());
    state.pages.insert(target_id.clone(), page);
    state.element_refs.clear();
    state.snapshot_url = None;

    Ok(format!("New page created: {} (url: {})", target_id, url))
}

pub(super) async fn action_select_page(args: &Value) -> Result<String> {
    require_browser().await?;
    let page_id =
        get_str(args, "page_id").ok_or_else(|| anyhow::anyhow!("Missing 'page_id' parameter"))?;

    let mut state = get_browser_state().lock().await;

    if !state.pages.contains_key(page_id) {
        let available: Vec<&String> = state.pages.keys().collect();
        return Err(anyhow::anyhow!(
            "Page '{}' not found. Available pages: {:?}",
            page_id,
            available
        ));
    }

    state.active_page_id = Some(page_id.to_string());
    state.element_refs.clear();
    state.snapshot_url = None;

    Ok(format!("Switched to page: {}", page_id))
}

pub(super) async fn action_close_page(args: &Value) -> Result<String> {
    require_browser().await?;
    let page_id =
        get_str(args, "page_id").ok_or_else(|| anyhow::anyhow!("Missing 'page_id' parameter"))?;

    let mut state = get_browser_state().lock().await;

    let page = state
        .pages
        .remove(page_id)
        .ok_or_else(|| anyhow::anyhow!("Page '{}' not found", page_id))?;

    let _ = page.close().await;

    if state.active_page_id.as_deref() == Some(page_id) {
        state.active_page_id = state.pages.keys().next().cloned();
        state.element_refs.clear();
        state.snapshot_url = None;
    }

    Ok(format!("Page '{}' closed.", page_id))
}
