//! Feishu approval (审批 / Lark Approval) — 5 LLM tools.
//!
//! - [`feishu_approval_create_instance`] — submit a new approval (HIGH risk)
//! - [`feishu_approval_get_instance`] — fetch instance detail
//! - [`feishu_approval_cancel_instance`] — withdraw an instance (HIGH risk)
//! - [`feishu_approval_list_instances`] — paginated instance code list
//! - [`feishu_approval_subscribe`] — enable event push (no-op until B.2)

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::tools::definitions::{ToolDefinition, ToolTier};

use super::resolve_feishu_api;

pub const TOOL_APPROVAL_CREATE_INSTANCE: &str = "feishu_approval_create_instance";
pub const TOOL_APPROVAL_GET_INSTANCE: &str = "feishu_approval_get_instance";
pub const TOOL_APPROVAL_CANCEL_INSTANCE: &str = "feishu_approval_cancel_instance";
pub const TOOL_APPROVAL_LIST_INSTANCES: &str = "feishu_approval_list_instances";
pub const TOOL_APPROVAL_SUBSCRIBE: &str = "feishu_approval_subscribe";

const CONFIG_HINT: &str =
    "Configure a Feishu IM channel account in Settings → Channels to enable approval tools.";

fn account_param() -> Value {
    json!({
        "type": "string",
        "description": "Feishu channel account ID. Required only when more than one Feishu account is configured."
    })
}

fn cfg() -> ToolTier {
    ToolTier::Configured {
        default_for_main: false,
        default_for_others: false,
        default_deferred: true,
        config_hint: CONFIG_HINT,
    }
}

pub fn create_instance_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_APPROVAL_CREATE_INSTANCE.into(),
        description:
            "**HIGH RISK** — start a new Feishu (Lark) approval instance. The instance reaches \
             the configured approvers' inbox immediately. ALWAYS confirm with the user before \
             calling, and surface the exact `approval_code` + form fields you intend to submit. \
             `form` is a JSON-encoded string per Feishu's approval form schema (typically a \
             stringified array of `{id, type, value}`). Required Feishu app scope: \
             `approval:approval`."
                .into(),
        tier: cfg(),
        internal: false,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "approval_code": {"type": "string", "description": "The approval definition code."},
                "user_id": {"type": "string", "description": "Submitter open_id (open_id format)."},
                "form": {"type": "string", "description": "JSON-encoded form fields per the approval definition's schema."},
                "account": account_param(),
            },
            "required": ["approval_code", "user_id", "form"],
            "additionalProperties": false
        }),
    }
}

pub fn get_instance_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_APPROVAL_GET_INSTANCE.into(),
        description:
            "Fetch a Feishu (Lark) approval instance's full state: status (`PENDING` / \
             `APPROVED` / `REJECTED` / `CANCELED` / etc.), submitted form, timeline, and task \
             list. Required Feishu app scope: `approval:approval.readonly` or `approval:approval`."
                .into(),
        tier: cfg(),
        internal: false,
        concurrent_safe: true,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "instance_code": {"type": "string"},
                "account": account_param(),
            },
            "required": ["instance_code"],
            "additionalProperties": false
        }),
    }
}

pub fn cancel_instance_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_APPROVAL_CANCEL_INSTANCE.into(),
        description:
            "**HIGH RISK** — withdraw a Feishu (Lark) approval instance. Only the original \
             submitter (or an admin) can cancel. Confirm with the user before calling and show \
             the instance's current status first. Required Feishu app scope: `approval:approval`."
                .into(),
        tier: cfg(),
        internal: false,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "approval_code": {"type": "string"},
                "instance_code": {"type": "string"},
                "user_id": {"type": "string", "description": "open_id of the original submitter."},
                "account": account_param(),
            },
            "required": ["approval_code", "instance_code", "user_id"],
            "additionalProperties": false
        }),
    }
}

pub fn list_instances_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_APPROVAL_LIST_INSTANCES.into(),
        description:
            "List approval instance codes for a given approval definition, with optional time \
             range. Returns just instance codes — fetch each via `feishu_approval_get_instance` \
             for full state. `start_time` / `end_time` are epoch-ms strings. Required Feishu app \
             scope: `approval:approval.readonly` or `approval:approval`."
                .into(),
        tier: cfg(),
        internal: false,
        concurrent_safe: true,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "approval_code": {"type": "string"},
                "start_time": {"type": "string", "description": "Epoch-ms string."},
                "end_time":   {"type": "string", "description": "Epoch-ms string."},
                "page_token": {"type": "string"},
                "page_size":  {"type": "integer"},
                "account": account_param(),
            },
            "required": ["approval_code"],
            "additionalProperties": false
        }),
    }
}

pub fn subscribe_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_APPROVAL_SUBSCRIBE.into(),
        description:
            "Enable Feishu event push for an approval definition. NOTE: hope-agent v0.2.0 \
             receives the events at the channel WebSocket gateway but only logs them — full \
             behavior (e.g. injecting approval-status changes back into a chat session) is \
             deferred to v0.3+. Idempotent: calling on an already-subscribed approval is safe. \
             Required Feishu app scope: `approval:approval`."
                .into(),
        tier: cfg(),
        internal: false,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "approval_code": {"type": "string"},
                "account": account_param(),
            },
            "required": ["approval_code"],
            "additionalProperties": false
        }),
    }
}

fn s<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k).and_then(|v| v.as_str())
}
fn r<'a>(args: &'a Value, k: &str) -> Result<&'a str> {
    s(args, k).ok_or_else(|| anyhow!("`{}` is required and must be a string", k))
}
fn u32_opt(args: &Value, k: &str) -> Result<Option<u32>> {
    match args.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|x| u32::try_from(x).ok())
            .map(Some)
            .ok_or_else(|| anyhow!("`{}` must fit u32", k)),
        _ => Err(anyhow!("`{}` must be an integer", k)),
    }
}

pub(crate) async fn execute_create_instance(args: &Value) -> Result<String> {
    let api = resolve_feishu_api(s(args, "account")).await?;
    let res = api
        .approval_create_instance(r(args, "approval_code")?, r(args, "user_id")?, r(args, "form")?)
        .await?;
    Ok(serde_json::to_string(&res)?)
}

pub(crate) async fn execute_get_instance(args: &Value) -> Result<String> {
    let api = resolve_feishu_api(s(args, "account")).await?;
    let inst = api.approval_get_instance(r(args, "instance_code")?).await?;
    Ok(serde_json::to_string(&inst)?)
}

pub(crate) async fn execute_cancel_instance(args: &Value) -> Result<String> {
    let api = resolve_feishu_api(s(args, "account")).await?;
    api.approval_cancel_instance(
        r(args, "approval_code")?,
        r(args, "instance_code")?,
        r(args, "user_id")?,
    )
    .await?;
    Ok(serde_json::json!({"ok": true}).to_string())
}

pub(crate) async fn execute_list_instances(args: &Value) -> Result<String> {
    let api = resolve_feishu_api(s(args, "account")).await?;
    let list = api
        .approval_list_instances(
            r(args, "approval_code")?,
            s(args, "start_time"),
            s(args, "end_time"),
            s(args, "page_token"),
            u32_opt(args, "page_size")?,
        )
        .await?;
    Ok(serde_json::to_string(&list)?)
}

pub(crate) async fn execute_subscribe(args: &Value) -> Result<String> {
    let api = resolve_feishu_api(s(args, "account")).await?;
    api.approval_subscribe(r(args, "approval_code")?).await?;
    Ok(serde_json::json!({"ok": true}).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match() {
        assert_eq!(create_instance_tool().name, TOOL_APPROVAL_CREATE_INSTANCE);
        assert_eq!(get_instance_tool().name, TOOL_APPROVAL_GET_INSTANCE);
        assert_eq!(cancel_instance_tool().name, TOOL_APPROVAL_CANCEL_INSTANCE);
        assert_eq!(list_instances_tool().name, TOOL_APPROVAL_LIST_INSTANCES);
        assert_eq!(subscribe_tool().name, TOOL_APPROVAL_SUBSCRIBE);
    }

    #[test]
    fn create_and_cancel_descriptions_flag_high_risk() {
        assert!(create_instance_tool().description.contains("HIGH"));
        assert!(cancel_instance_tool().description.contains("HIGH"));
    }

    #[tokio::test]
    async fn create_requires_all_fields() {
        let err = execute_create_instance(&json!({})).await.unwrap_err();
        assert!(err.to_string().contains("approval_code"), "{}", err);
    }
}
