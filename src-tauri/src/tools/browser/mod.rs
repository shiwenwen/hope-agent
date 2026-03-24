use anyhow::Result;
use serde_json::Value;

use crate::browser_state;


mod connection;
mod navigation;
mod snapshot;
mod interaction;
mod advanced;

/// Image base64 prefix marker — detected by agent.rs for multimodal content
pub const IMAGE_BASE64_PREFIX: &str = "__IMAGE_BASE64__";

pub(crate) async fn tool_browser(args: &Value) -> Result<String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

    match action {
        "connect" => connection::action_connect(args).await,
        "launch" => connection::action_launch(args).await,
        "disconnect" => connection::action_disconnect().await,
        "list_pages" => navigation::action_list_pages().await,
        "new_page" => navigation::action_new_page(args).await,
        "select_page" => navigation::action_select_page(args).await,
        "close_page" => navigation::action_close_page(args).await,
        "navigate" => navigation::action_navigate(args).await,
        "go_back" => navigation::action_go_back().await,
        "go_forward" => navigation::action_go_forward().await,
        "take_snapshot" => snapshot::action_take_snapshot().await,
        "take_screenshot" => snapshot::action_take_screenshot(args).await,
        "click" => interaction::action_click(args).await,
        "fill" => interaction::action_fill(args).await,
        "fill_form" => interaction::action_fill_form(args).await,
        "hover" => interaction::action_hover(args).await,
        "drag" => interaction::action_drag(args).await,
        "press_key" => interaction::action_press_key(args).await,
        "upload_file" => interaction::action_upload_file(args).await,
        "evaluate" => advanced::action_evaluate(args).await,
        "wait_for" => advanced::action_wait_for(args).await,
        "handle_dialog" => advanced::action_handle_dialog(args).await,
        "resize" => advanced::action_resize(args).await,
        "scroll" => advanced::action_scroll(args).await,
        "list_profiles" => advanced::action_list_profiles().await,
        "save_pdf" => advanced::action_save_pdf(args).await,
        _ => Err(anyhow::anyhow!(
            "Unknown browser action: '{}'. Available: connect, launch, disconnect, list_pages, new_page, select_page, close_page, navigate, go_back, go_forward, take_snapshot, take_screenshot, click, fill, fill_form, hover, drag, press_key, upload_file, evaluate, wait_for, handle_dialog, resize, scroll, list_profiles, save_pdf",
            action
        )),
    }
}

// ── Helpers ──────────────────────────────────────────────────────

async fn require_browser() -> Result<()> {
    browser_state::ensure_connected().await
}

fn get_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| {
        v.as_str().or_else(|| v.get("text").and_then(|t| t.as_str()))
    })
}

fn get_u32(args: &Value, key: &str) -> Option<u32> {
    args.get(key).and_then(|v| v.as_u64()).map(|v| v as u32)
}

fn get_i64(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(|v| v.as_i64())
}

fn get_bool(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(|v| v.as_bool())
}
