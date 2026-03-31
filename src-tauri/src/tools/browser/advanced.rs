use anyhow::Result;
use serde_json::Value;

use super::{get_bool, get_i64, get_str, get_u32, require_browser};
use crate::browser_state::get_browser_state;

pub(super) async fn action_evaluate(args: &Value) -> Result<String> {
    require_browser().await?;
    let expression = get_str(args, "expression")
        .ok_or_else(|| anyhow::anyhow!("Missing 'expression' parameter"))?;

    let state = get_browser_state().lock().await;
    let page = state.get_active_page()?;

    let result = page
        .evaluate(expression)
        .await
        .map_err(|e| anyhow::anyhow!("Script evaluation failed: {}", e))?;

    // Try to extract as string, then fall back to JSON
    let value: serde_json::Value = result.into_value().unwrap_or(serde_json::Value::Null);

    let display = if value.is_string() {
        value.as_str().unwrap_or("").to_string()
    } else if value.is_null() {
        "undefined".to_string()
    } else {
        serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
    };

    Ok(format!("Result: {}", display))
}

pub(super) async fn action_wait_for(args: &Value) -> Result<String> {
    require_browser().await?;
    let text = get_str(args, "text").ok_or_else(|| anyhow::anyhow!("Missing 'text' parameter"))?;
    let timeout_ms = get_u32(args, "timeout").unwrap_or(30000) as u64;

    let check_js = format!(
        "document.body.innerText.includes('{}')",
        text.replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('\n', "\\n")
    );

    let start = std::time::Instant::now();
    let poll_interval = std::time::Duration::from_millis(500);

    loop {
        {
            let state = get_browser_state().lock().await;
            let page = state.get_active_page()?;

            let found: bool = page
                .evaluate(check_js.as_str())
                .await
                .ok()
                .and_then(|r| r.into_value().ok())
                .unwrap_or(false);

            if found {
                return Ok(format!("Text \"{}\" found on page.", text));
            }
        }

        if start.elapsed().as_millis() as u64 >= timeout_ms {
            return Err(anyhow::anyhow!(
                "Timeout after {}ms waiting for text \"{}\"",
                timeout_ms,
                text
            ));
        }

        tokio::time::sleep(poll_interval).await;
    }
}

pub(super) async fn action_handle_dialog(args: &Value) -> Result<String> {
    require_browser().await?;
    let accept = get_bool(args, "accept").ok_or_else(|| {
        anyhow::anyhow!("Missing 'accept' parameter (true to accept, false to dismiss)")
    })?;
    let dialog_text = get_str(args, "dialog_text");

    let state = get_browser_state().lock().await;
    let page = state.get_active_page()?;

    use chromiumoxide::cdp::browser_protocol::page::HandleJavaScriptDialogParams;

    let mut params = HandleJavaScriptDialogParams::new(accept);
    if let Some(text) = dialog_text {
        params.prompt_text = Some(text.to_string());
    }

    page.execute(params)
        .await
        .map_err(|e| anyhow::anyhow!("Handle dialog failed: {}. Is there a dialog open?", e))?;

    Ok(format!(
        "Dialog {}.{}",
        if accept { "accepted" } else { "dismissed" },
        dialog_text
            .map(|t| format!(" Prompt text: \"{}\"", t))
            .unwrap_or_default()
    ))
}

pub(super) async fn action_resize(args: &Value) -> Result<String> {
    require_browser().await?;
    let width =
        get_u32(args, "width").ok_or_else(|| anyhow::anyhow!("Missing 'width' parameter"))? as i64;
    let height = get_u32(args, "height")
        .ok_or_else(|| anyhow::anyhow!("Missing 'height' parameter"))? as i64;

    let state = get_browser_state().lock().await;
    let page = state.get_active_page()?;

    use chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams;

    let params = SetDeviceMetricsOverrideParams::new(width, height, 1.0, false);
    page.execute(params)
        .await
        .map_err(|e| anyhow::anyhow!("Resize failed: {}", e))?;

    Ok(format!("Viewport resized to {}x{}", width, height))
}

pub(super) async fn action_scroll(args: &Value) -> Result<String> {
    require_browser().await?;
    let direction = get_str(args, "direction").unwrap_or("down");
    let amount = get_i64(args, "amount").unwrap_or(500);

    let state = get_browser_state().lock().await;
    let page = state.get_active_page()?;

    let (dx, dy) = match direction {
        "up" => (0, -amount),
        "down" => (0, amount),
        "left" => (-amount, 0),
        "right" => (amount, 0),
        _ => {
            return Err(anyhow::anyhow!(
                "Invalid direction: '{}'. Use: up, down, left, right",
                direction
            ))
        }
    };

    let js = format!("window.scrollBy({}, {})", dx, dy);
    page.evaluate(js)
        .await
        .map_err(|e| anyhow::anyhow!("Scroll failed: {}", e))?;

    Ok(format!("Scrolled {} by {} pixels", direction, amount.abs()))
}

pub(super) async fn action_save_pdf(args: &Value) -> Result<String> {
    require_browser().await?;

    let state = get_browser_state().lock().await;
    let page = state.get_active_page()?;

    use chromiumoxide::cdp::browser_protocol::page::PrintToPdfParams;

    let mut params = PrintToPdfParams::default();

    // Optional paper format
    if let Some(paper) = get_str(args, "paper_format") {
        let (w, h) = match paper {
            "a3" | "A3" => (11.69, 16.54),
            "a4" | "A4" => (8.27, 11.69),
            "a5" | "A5" => (5.83, 8.27),
            "letter" => (8.5, 11.0),
            "legal" => (8.5, 14.0),
            "tabloid" => (11.0, 17.0),
            _ => {
                return Err(anyhow::anyhow!(
                    "Unknown paper_format: '{}'. Options: a3, a4, a5, letter, legal, tabloid",
                    paper
                ))
            }
        };
        params.paper_width = Some(w);
        params.paper_height = Some(h);
    }

    // Optional landscape
    if let Some(landscape) = get_bool(args, "landscape") {
        params.landscape = Some(landscape);
    }

    // Optional print background
    if let Some(bg) = get_bool(args, "print_background") {
        params.print_background = Some(bg);
    }

    let pdf_bytes = page.pdf(params).await
        .map_err(|e| anyhow::anyhow!("PDF export failed: {}. Note: PDF generation requires Chrome to NOT be in non-headless mode with some configurations.", e))?;

    // Determine output path
    let output_path = if let Some(path) = get_str(args, "output_path") {
        std::path::PathBuf::from(path)
    } else {
        // Default: save to ~/.opencomputer/share/ with timestamp
        let share_dir = crate::paths::share_dir()?;
        std::fs::create_dir_all(&share_dir)?;
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        share_dir.join(format!("page_{}.pdf", timestamp))
    };

    // Ensure parent directory exists
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(&output_path, &pdf_bytes)?;

    let url = page
        .url()
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "unknown".to_string());

    Ok(format!(
        "PDF saved: {} ({} bytes)\nSource: {}",
        output_path.display(),
        pdf_bytes.len(),
        url
    ))
}

pub(super) async fn action_list_profiles() -> Result<String> {
    let profiles_dir = crate::paths::browser_profiles_dir()?;

    if !profiles_dir.exists() {
        return Ok("No browser profiles found. Use action='launch' with 'profile' parameter to create one.".to_string());
    }

    let mut profiles = Vec::new();
    for entry in std::fs::read_dir(&profiles_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            profiles.push(name);
        }
    }

    if profiles.is_empty() {
        return Ok("No browser profiles found. Use action='launch' with 'profile' parameter to create one.".to_string());
    }

    profiles.sort();

    // Show which profile is currently active
    let state = get_browser_state().lock().await;
    let active_profile = state.profile.as_deref();

    let mut lines = vec![format!("Browser profiles ({}):", profiles.len())];
    for name in &profiles {
        let marker = if active_profile == Some(name.as_str()) {
            " [active]"
        } else {
            ""
        };
        lines.push(format!("  - {}{}", name, marker));
    }

    Ok(lines.join("\n"))
}
