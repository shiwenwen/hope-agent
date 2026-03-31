use anyhow::Result;
use serde_json::Value;

/// Send a native desktop notification to the user via Tauri event.
/// The frontend handles the actual macOS notification dispatch, ensuring
/// consistent permission checks and global toggle enforcement.
pub(crate) async fn tool_send_notification(
    args: &Value,
    _ctx: &super::ToolExecContext,
) -> Result<String> {
    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("OpenComputer");
    let body = args
        .get("body")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: body"))?;

    if let Some(handle) = crate::get_app_handle() {
        use tauri::Emitter;
        let payload = serde_json::json!({
            "type": "agent_notification",
            "title": title,
            "body": body,
        });
        let _ = handle.emit("agent:send_notification", payload);
    }

    Ok(format!("Notification sent: {} - {}", title, body))
}
