use anyhow::Result;
use serde_json::Value;

/// Send a native desktop notification to the user via Tauri event.
/// The frontend handles the actual macOS notification dispatch, ensuring
/// consistent permission checks and global toggle enforcement.
pub(crate) async fn tool_send_notification(
    args: &Value,
    ctx: &super::ToolExecContext,
) -> Result<String> {
    if !crate::personal_assistant::attention::signals::classify_context(ctx)
        .await
        .is_ordinary()
    {
        return Ok("Notification folded into Hope. Report useful results or a pending decision through the current item; unified attention chooses whether and when to notify.".into());
    }
    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Hope Agent");
    let body = args
        .get("body")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: body"))?;

    if let Some(bus) = crate::globals::get_event_bus() {
        let payload = serde_json::json!({
            "type": "agent_notification",
            "title": title,
            "body": body,
        });
        bus.emit("agent:send_notification", payload);
    }

    Ok(format!("Notification sent: {} - {}", title, body))
}
