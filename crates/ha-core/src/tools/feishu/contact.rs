//! Feishu contact (联系人 / Lark Contact) — 4 LLM tools.
//!
//! **Sensitive data warning**: returns employee personal info (name /
//! email / mobile / department / job title / avatar). Surface this in
//! the tool descriptions so the agent treats results carefully.

use anyhow::Result;
use serde_json::{json, Value};

use crate::tools::definitions::{ToolDefinition, ToolTier};

use super::{
    account_param, arg_required_str, arg_required_string_array, arg_str, arg_u32, configured_tier,
    resolve_feishu_api,
};

pub const TOOL_CONTACT_GET_USER: &str = "feishu_contact_get_user";
pub const TOOL_CONTACT_BATCH_GET_USERS: &str = "feishu_contact_batch_get_users";
pub const TOOL_CONTACT_GET_DEPARTMENT: &str = "feishu_contact_get_department";
pub const TOOL_CONTACT_SEARCH_USERS_BY_DEPARTMENT: &str =
    "feishu_contact_search_users_by_department";

const HINT: &str =
    "Configure a Feishu IM channel account in Settings → Channels to enable contact tools.";

const SENSITIVE_HINT: &str = " ⚠️ Returns employee personal info (name / email / mobile / department / job_title / avatar). Treat results as sensitive — don't echo verbatim into untrusted contexts.";

fn cfg() -> ToolTier {
    configured_tier(HINT)
}

pub fn get_user_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_CONTACT_GET_USER.into(),
        description: format!(
            "Fetch a Feishu (Lark) user's profile by user_id. Required Feishu app scope: \
             `contact:user.id:readonly` or stronger.{}",
            SENSITIVE_HINT
        ),
        tier: cfg(),
        internal: false,
        concurrent_safe: true,
        background_policy: crate::tools::definitions::BackgroundPolicy::ForegroundOnly,
        parameters: json!({
            "type": "object",
            "properties": {
                "user_id": {"type": "string"},
                "user_id_type": {"type": "string", "description": "`open_id` (default) / `union_id` / `user_id`."},
                "account": account_param(),
            },
            "required": ["user_id"],
            "additionalProperties": false
        }),
    }
}

pub fn batch_get_users_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_CONTACT_BATCH_GET_USERS.into(),
        description: format!(
            "Fetch up to 50 Feishu (Lark) users in one call. Required Feishu app scope: \
             `contact:user.id:readonly` or stronger.{}",
            SENSITIVE_HINT
        ),
        tier: cfg(),
        internal: false,
        concurrent_safe: true,
        background_policy: crate::tools::definitions::BackgroundPolicy::ForegroundOnly,
        parameters: json!({
            "type": "object",
            "properties": {
                "user_ids": {"type": "array", "items": {"type": "string"}, "description": "1-50 user IDs."},
                "user_id_type": {"type": "string"},
                "account": account_param(),
            },
            "required": ["user_ids"],
            "additionalProperties": false
        }),
    }
}

pub fn get_department_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_CONTACT_GET_DEPARTMENT.into(),
        description:
            "Fetch a Feishu (Lark) department's info by department_id (name, parent, member \
             count, leader). Required Feishu app scope: `contact:department.id:readonly` or \
             stronger."
                .into(),
        tier: cfg(),
        internal: false,
        concurrent_safe: true,
        background_policy: crate::tools::definitions::BackgroundPolicy::ForegroundOnly,
        parameters: json!({
            "type": "object",
            "properties": {
                "department_id": {"type": "string"},
                "department_id_type": {"type": "string", "description": "`open_department_id` (default) / `department_id`."},
                "account": account_param(),
            },
            "required": ["department_id"],
            "additionalProperties": false
        }),
    }
}

pub fn search_users_by_department_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_CONTACT_SEARCH_USERS_BY_DEPARTMENT.into(),
        description: format!(
            "List Feishu (Lark) users belonging to a department, paginated. Useful for \
             enumerating a team. Required Feishu app scope: `contact:user.id:readonly` or \
             stronger.{}",
            SENSITIVE_HINT
        ),
        tier: cfg(),
        internal: false,
        concurrent_safe: true,
        background_policy: crate::tools::definitions::BackgroundPolicy::ForegroundOnly,
        parameters: json!({
            "type": "object",
            "properties": {
                "department_id": {"type": "string"},
                "page_token": {"type": "string"},
                "page_size": {"type": "integer"},
                "account": account_param(),
            },
            "required": ["department_id"],
            "additionalProperties": false
        }),
    }
}

pub(crate) async fn execute_get_user(args: &Value) -> Result<String> {
    let api = resolve_feishu_api(arg_str(args, "account")).await?;
    let u = api
        .contact_get_user(
            arg_required_str(args, "user_id")?,
            arg_str(args, "user_id_type"),
        )
        .await?;
    Ok(serde_json::to_string(&u)?)
}
pub(crate) async fn execute_batch_get_users(args: &Value) -> Result<String> {
    let user_ids = arg_required_string_array(args, "user_ids")?;
    let api = resolve_feishu_api(arg_str(args, "account")).await?;
    let r = api
        .contact_batch_get_users(user_ids, arg_str(args, "user_id_type"))
        .await?;
    Ok(serde_json::to_string(&r)?)
}
pub(crate) async fn execute_get_department(args: &Value) -> Result<String> {
    let api = resolve_feishu_api(arg_str(args, "account")).await?;
    let d = api
        .contact_get_department(
            arg_required_str(args, "department_id")?,
            arg_str(args, "department_id_type"),
        )
        .await?;
    Ok(serde_json::to_string(&d)?)
}
pub(crate) async fn execute_search_users_by_department(args: &Value) -> Result<String> {
    let api = resolve_feishu_api(arg_str(args, "account")).await?;
    let r = api
        .contact_search_users_by_department(
            crate::channel::feishu::api_contact::ContactSearchUsersByDepartmentReq {
                department_id: arg_required_str(args, "department_id")?,
                page_token: arg_str(args, "page_token"),
                page_size: arg_u32(args, "page_size")?,
            },
        )
        .await?;
    Ok(serde_json::to_string(&r)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_match() {
        assert_eq!(get_user_tool().name, TOOL_CONTACT_GET_USER);
        assert_eq!(batch_get_users_tool().name, TOOL_CONTACT_BATCH_GET_USERS);
        assert_eq!(get_department_tool().name, TOOL_CONTACT_GET_DEPARTMENT);
        assert_eq!(
            search_users_by_department_tool().name,
            TOOL_CONTACT_SEARCH_USERS_BY_DEPARTMENT
        );
    }
    #[test]
    fn user_tools_flag_sensitive_data() {
        assert!(get_user_tool().description.contains("personal info"));
        assert!(batch_get_users_tool().description.contains("personal info"));
        assert!(search_users_by_department_tool()
            .description
            .contains("personal info"));
    }
    #[tokio::test]
    async fn batch_requires_user_ids_array() {
        let err = execute_batch_get_users(&json!({})).await.unwrap_err();
        assert!(err.to_string().contains("user_ids"), "{}", err);
    }
}
