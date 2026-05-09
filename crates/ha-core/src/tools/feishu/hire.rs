//! Feishu hire (招聘 / Lark Hire) — 5 LLM tools.
//!
//! Tenant must have the hire module enabled; otherwise calls return
//! Feishu code `1061004` ("module not enabled"). Tool descriptions
//! surface this so the LLM can guide the user to the admin panel.

use anyhow::Result;
use serde_json::{json, Value};

use crate::tools::definitions::{ToolDefinition, ToolTier};

use super::{
    account_param, arg_required_str, arg_str, arg_u32, configured_tier, resolve_feishu_api,
};

pub const TOOL_HIRE_LIST_JOBS: &str = "feishu_hire_list_jobs";
pub const TOOL_HIRE_GET_JOB: &str = "feishu_hire_get_job";
pub const TOOL_HIRE_LIST_TALENTS: &str = "feishu_hire_list_talents";
pub const TOOL_HIRE_GET_TALENT: &str = "feishu_hire_get_talent";
pub const TOOL_HIRE_LIST_APPLICATIONS: &str = "feishu_hire_list_applications";

const HINT: &str =
    "Configure a Feishu IM channel account in Settings → Channels to enable hire tools.";

const MODULE_HINT: &str = " Note: Feishu's hire module must be enabled for this tenant — error code `1061004` means the admin needs to enable hire in the workspace settings first.";

fn cfg() -> ToolTier {
    configured_tier(HINT)
}

fn pagination_only(extra_required: &[&str]) -> Value {
    let mut props = serde_json::Map::new();
    props.insert("page_token".into(), json!({"type": "string"}));
    props.insert("page_size".into(), json!({"type": "integer"}));
    props.insert("account".into(), account_param());
    json!({
        "type": "object",
        "properties": props,
        "required": extra_required,
        "additionalProperties": false
    })
}

pub fn list_jobs_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_HIRE_LIST_JOBS.into(),
        description: format!(
            "List Feishu (Lark) hire job postings, paginated. Required Feishu app scope: \
             `hire:job:readonly`.{}",
            MODULE_HINT
        ),
        tier: cfg(),
        internal: false,
        concurrent_safe: true,
        async_capable: false,
        parameters: pagination_only(&[]),
    }
}

pub fn get_job_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_HIRE_GET_JOB.into(),
        description: format!(
            "Fetch a single Feishu (Lark) hire job posting (title / description / requirements / \
             owner). Required Feishu app scope: `hire:job:readonly`.{}",
            MODULE_HINT
        ),
        tier: cfg(),
        internal: false,
        concurrent_safe: true,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "job_id": {"type": "string"},
                "account": account_param(),
            },
            "required": ["job_id"],
            "additionalProperties": false
        }),
    }
}

pub fn list_talents_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_HIRE_LIST_TALENTS.into(),
        description: format!(
            "List talents in the Feishu (Lark) hire talent pool, paginated. ⚠️ Returns candidate \
             personal info — treat as sensitive. Required Feishu app scope: \
             `hire:talent:readonly`.{}",
            MODULE_HINT
        ),
        tier: cfg(),
        internal: false,
        concurrent_safe: true,
        async_capable: false,
        parameters: pagination_only(&[]),
    }
}

pub fn get_talent_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_HIRE_GET_TALENT.into(),
        description: format!(
            "Fetch a single talent's profile from the Feishu (Lark) hire talent pool. ⚠️ \
             Returns full candidate info (name / contacts / résumé). Required Feishu app scope: \
             `hire:talent:readonly`.{}",
            MODULE_HINT
        ),
        tier: cfg(),
        internal: false,
        concurrent_safe: true,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "talent_id": {"type": "string"},
                "account": account_param(),
            },
            "required": ["talent_id"],
            "additionalProperties": false
        }),
    }
}

pub fn list_applications_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_HIRE_LIST_APPLICATIONS.into(),
        description: format!(
            "List Feishu (Lark) hire applications (talent → job submissions), paginated. \
             Required Feishu app scope: `hire:application:readonly`.{}",
            MODULE_HINT
        ),
        tier: cfg(),
        internal: false,
        concurrent_safe: true,
        async_capable: false,
        parameters: pagination_only(&[]),
    }
}

pub(crate) async fn execute_list_jobs(args: &Value) -> Result<String> {
    let api = resolve_feishu_api(arg_str(args, "account")).await?;
    let r = api
        .hire_list_jobs(arg_str(args, "page_token"), arg_u32(args, "page_size")?)
        .await?;
    Ok(serde_json::to_string(&r)?)
}
pub(crate) async fn execute_get_job(args: &Value) -> Result<String> {
    let api = resolve_feishu_api(arg_str(args, "account")).await?;
    let r = api.hire_get_job(arg_required_str(args, "job_id")?).await?;
    Ok(serde_json::to_string(&r)?)
}
pub(crate) async fn execute_list_talents(args: &Value) -> Result<String> {
    let api = resolve_feishu_api(arg_str(args, "account")).await?;
    let r = api
        .hire_list_talents(arg_str(args, "page_token"), arg_u32(args, "page_size")?)
        .await?;
    Ok(serde_json::to_string(&r)?)
}
pub(crate) async fn execute_get_talent(args: &Value) -> Result<String> {
    let api = resolve_feishu_api(arg_str(args, "account")).await?;
    let r = api
        .hire_get_talent(arg_required_str(args, "talent_id")?)
        .await?;
    Ok(serde_json::to_string(&r)?)
}
pub(crate) async fn execute_list_applications(args: &Value) -> Result<String> {
    let api = resolve_feishu_api(arg_str(args, "account")).await?;
    let r = api
        .hire_list_applications(arg_str(args, "page_token"), arg_u32(args, "page_size")?)
        .await?;
    Ok(serde_json::to_string(&r)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_match() {
        assert_eq!(list_jobs_tool().name, TOOL_HIRE_LIST_JOBS);
        assert_eq!(get_job_tool().name, TOOL_HIRE_GET_JOB);
        assert_eq!(list_talents_tool().name, TOOL_HIRE_LIST_TALENTS);
        assert_eq!(get_talent_tool().name, TOOL_HIRE_GET_TALENT);
        assert_eq!(list_applications_tool().name, TOOL_HIRE_LIST_APPLICATIONS);
    }
    #[test]
    fn all_descriptions_warn_about_hire_module() {
        for d in [
            list_jobs_tool(),
            get_job_tool(),
            list_talents_tool(),
            get_talent_tool(),
            list_applications_tool(),
        ] {
            assert!(d.description.contains("1061004"), "{}", d.name);
        }
    }
}
