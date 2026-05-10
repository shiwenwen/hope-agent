//! Feishu calendar (日历 / Lark Calendar) — 6 LLM tools.

use anyhow::Result;
use serde_json::{json, Value};

use crate::tools::definitions::{ToolDefinition, ToolTier};

use super::{
    account_param, arg_required_array, arg_required_object, arg_required_str, arg_str, arg_u32,
    configured_tier, resolve_feishu_api,
};

pub const TOOL_CALENDAR_LIST: &str = "feishu_calendar_list";
pub const TOOL_CALENDAR_CREATE_EVENT: &str = "feishu_calendar_create_event";
pub const TOOL_CALENDAR_LIST_EVENTS: &str = "feishu_calendar_list_events";
pub const TOOL_CALENDAR_UPDATE_EVENT: &str = "feishu_calendar_update_event";
pub const TOOL_CALENDAR_DELETE_EVENT: &str = "feishu_calendar_delete_event";
pub const TOOL_CALENDAR_ATTENDEES_CREATE: &str = "feishu_calendar_attendees_create";

const HINT: &str =
    "Configure a Feishu IM channel account in Settings → Channels to enable calendar tools.";

fn cfg() -> ToolTier {
    configured_tier(HINT)
}

pub fn list_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_CALENDAR_LIST.into(),
        description:
            "List Feishu (Lark) calendars accessible to the bot. Returns calendar IDs and \
             metadata. Required Feishu app scope: `calendar:calendar.readonly` or \
             `calendar:calendar`."
                .into(),
        tier: cfg(),
        internal: false,
        concurrent_safe: true,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "page_token": {"type": "string"},
                "page_size":  {"type": "integer"},
                "account": account_param(),
            },
            "additionalProperties": false
        }),
    }
}

pub fn create_event_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_CALENDAR_CREATE_EVENT.into(),
        description:
            "Create a new event in a Feishu (Lark) calendar. `event` must follow Feishu's event \
             schema: typically `{summary, description?, start_time: {timestamp/date/timezone}, \
             end_time: {...}, attendees?, ...}`. Use ISO 8601 / RFC 3339 timestamps; convert \
             user-local times to the right timezone yourself. Required Feishu app scope: \
             `calendar:calendar`."
                .into(),
        tier: cfg(),
        internal: false,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "calendar_id": {"type": "string"},
                "event": {"type": "object", "description": "Full event JSON per Feishu's schema."},
                "account": account_param(),
            },
            "required": ["calendar_id", "event"],
            "additionalProperties": false
        }),
    }
}

pub fn list_events_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_CALENDAR_LIST_EVENTS.into(),
        description:
            "List events in a Feishu (Lark) calendar with optional time range. `start_time` / \
             `end_time` are RFC 3339 strings (or epoch-second strings). Required Feishu app \
             scope: `calendar:calendar.readonly` or `calendar:calendar`."
                .into(),
        tier: cfg(),
        internal: false,
        concurrent_safe: true,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "calendar_id": {"type": "string"},
                "start_time": {"type": "string"},
                "end_time": {"type": "string"},
                "page_token": {"type": "string"},
                "page_size": {"type": "integer"},
                "account": account_param(),
            },
            "required": ["calendar_id"],
            "additionalProperties": false
        }),
    }
}

pub fn update_event_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_CALENDAR_UPDATE_EVENT.into(),
        description:
            "Patch an existing Feishu (Lark) calendar event. Only the fields included in `patch` \
             are modified; all others are preserved. To reschedule, include both `start_time` and \
             `end_time` (Feishu rejects partial-time-range edits). Required Feishu app scope: \
             `calendar:calendar`."
                .into(),
        tier: cfg(),
        internal: false,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "calendar_id": {"type": "string"},
                "event_id": {"type": "string"},
                "patch": {"type": "object", "description": "Partial event JSON; replaces only the listed fields."},
                "account": account_param(),
            },
            "required": ["calendar_id", "event_id", "patch"],
            "additionalProperties": false
        }),
    }
}

pub fn delete_event_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_CALENDAR_DELETE_EVENT.into(),
        description:
            "Delete a Feishu (Lark) calendar event. Notifies all attendees automatically. \
             Required Feishu app scope: `calendar:calendar`."
                .into(),
        tier: cfg(),
        internal: false,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "calendar_id": {"type": "string"},
                "event_id": {"type": "string"},
                "account": account_param(),
            },
            "required": ["calendar_id", "event_id"],
            "additionalProperties": false
        }),
    }
}

pub fn attendees_create_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_CALENDAR_ATTENDEES_CREATE.into(),
        description:
            "Invite attendees to a Feishu (Lark) calendar event. `attendees` is the per-Feishu \
             schema array — each entry like `{type: \"user\"|\"chat\"|\"resource\"|\"third_party\", \
             user_id?: \"ou_...\", chat_id?: \"oc_...\", third_party_email?: \"...\", ...}`. \
             Required Feishu app scope: `calendar:calendar`."
                .into(),
        tier: cfg(),
        internal: false,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "calendar_id": {"type": "string"},
                "event_id": {"type": "string"},
                "attendees": {"type": "array", "items": {"type": "object"}},
                "account": account_param(),
            },
            "required": ["calendar_id", "event_id", "attendees"],
            "additionalProperties": false
        }),
    }
}

pub(crate) async fn execute_list(args: &Value) -> Result<String> {
    let api = resolve_feishu_api(arg_str(args, "account")).await?;
    let r = api
        .calendar_list(arg_str(args, "page_token"), arg_u32(args, "page_size")?)
        .await?;
    Ok(serde_json::to_string(&r)?)
}
pub(crate) async fn execute_create_event(args: &Value) -> Result<String> {
    let calendar_id = arg_required_str(args, "calendar_id")?;
    let event = arg_required_object(args, "event")?;
    let api = resolve_feishu_api(arg_str(args, "account")).await?;
    let r = api.calendar_create_event(calendar_id, event).await?;
    Ok(serde_json::to_string(&r)?)
}
pub(crate) async fn execute_list_events(args: &Value) -> Result<String> {
    let api = resolve_feishu_api(arg_str(args, "account")).await?;
    let r = api
        .calendar_list_events(crate::channel::feishu::api_calendar::CalendarListEventsReq {
            calendar_id: arg_required_str(args, "calendar_id")?,
            start_time: arg_str(args, "start_time"),
            end_time: arg_str(args, "end_time"),
            page_token: arg_str(args, "page_token"),
            page_size: arg_u32(args, "page_size")?,
        })
        .await?;
    Ok(serde_json::to_string(&r)?)
}
pub(crate) async fn execute_update_event(args: &Value) -> Result<String> {
    let api = resolve_feishu_api(arg_str(args, "account")).await?;
    let r = api
        .calendar_update_event(
            arg_required_str(args, "calendar_id")?,
            arg_required_str(args, "event_id")?,
            arg_required_object(args, "patch")?,
        )
        .await?;
    Ok(serde_json::to_string(&r)?)
}
pub(crate) async fn execute_delete_event(args: &Value) -> Result<String> {
    let api = resolve_feishu_api(arg_str(args, "account")).await?;
    api.calendar_delete_event(
        arg_required_str(args, "calendar_id")?,
        arg_required_str(args, "event_id")?,
    )
    .await?;
    Ok(serde_json::json!({"ok": true}).to_string())
}
pub(crate) async fn execute_attendees_create(args: &Value) -> Result<String> {
    let api = resolve_feishu_api(arg_str(args, "account")).await?;
    let r = api
        .calendar_attendees_create(
            arg_required_str(args, "calendar_id")?,
            arg_required_str(args, "event_id")?,
            arg_required_array(args, "attendees")?,
        )
        .await?;
    Ok(serde_json::to_string(&r)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_match() {
        assert_eq!(list_tool().name, TOOL_CALENDAR_LIST);
        assert_eq!(create_event_tool().name, TOOL_CALENDAR_CREATE_EVENT);
        assert_eq!(list_events_tool().name, TOOL_CALENDAR_LIST_EVENTS);
        assert_eq!(update_event_tool().name, TOOL_CALENDAR_UPDATE_EVENT);
        assert_eq!(delete_event_tool().name, TOOL_CALENDAR_DELETE_EVENT);
        assert_eq!(attendees_create_tool().name, TOOL_CALENDAR_ATTENDEES_CREATE);
    }
    #[tokio::test]
    async fn create_requires_event_object() {
        let err = execute_create_event(&json!({"calendar_id": "c"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("event"), "{}", err);
    }
}
