//! Feishu bitable (多维表格 / Lark Base) — 4 LLM tools.
//!
//! - [`feishu_bitable_list_records`]   — paginated list with optional view + filter expression
//! - [`feishu_bitable_search_records`] — structured query with field projection + sort
//! - [`feishu_bitable_create_record`]  — single-record insert
//! - [`feishu_bitable_batch_update_records`] — batch update (≤1000)
//!
//! All four go through [`super::resolve_feishu_api`] for account routing
//! and shared token cache. Tier 3 Configured / off-by-default. Required
//! Feishu app scope: `bitable:app` (or `bitable:app.read` for list /
//! search). Record `fields` is intentionally typed as object so callers
//! can pass any per-table column shape.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::channel::feishu::api_bitable::BITABLE_BATCH_UPDATE_MAX;
use crate::tools::definitions::{ToolDefinition, ToolTier};

use super::{
    account_param, arg_required_object, arg_required_str, arg_str, arg_u32, configured_tier,
    resolve_feishu_api,
};

pub const TOOL_BITABLE_LIST_RECORDS: &str = "feishu_bitable_list_records";
pub const TOOL_BITABLE_SEARCH_RECORDS: &str = "feishu_bitable_search_records";
pub const TOOL_BITABLE_CREATE_RECORD: &str = "feishu_bitable_create_record";
pub const TOOL_BITABLE_BATCH_UPDATE_RECORDS: &str = "feishu_bitable_batch_update_records";
pub const TOOL_BITABLE_LIST_VIEWS: &str = "feishu_bitable_list_views";
pub const TOOL_BITABLE_GET_VIEW: &str = "feishu_bitable_get_view";
pub const TOOL_BITABLE_LIST_DASHBOARDS: &str = "feishu_bitable_list_dashboards";

const CONFIG_HINT: &str =
    "Configure a Feishu IM channel account in Settings → Channels to enable bitable tools.";

fn cfg() -> ToolTier {
    configured_tier(CONFIG_HINT)
}

// ── Tool definitions ────────────────────────────────────────────

pub fn list_records_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_BITABLE_LIST_RECORDS.into(),
        description:
            "List records from a Feishu (Lark) bitable table, paginated. Use the simpler `filter` \
             expression (e.g. `CurrentValue.[Status]=\"Done\"`); for multi-condition queries with \
             field projection / sort, prefer `feishu_bitable_search_records`. Required Feishu app \
             scope: `bitable:app.read` or `bitable:app`."
                .into(),
        tier: cfg(),
        internal: false,
        concurrent_safe: true,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "app_token": {
                    "type": "string",
                    "description": "The bitable app token (e.g. `bascnAbc...`)."
                },
                "table_id": {
                    "type": "string",
                    "description": "Target table ID (e.g. `tblXyz...`)."
                },
                "view_id": {
                    "type": "string",
                    "description": "Optional view ID to constrain the listing to a specific view."
                },
                "filter": {
                    "type": "string",
                    "description": "Optional Feishu filter expression. Example: `CurrentValue.[Status]=\"Done\"`."
                },
                "page_token": {
                    "type": "string",
                    "description": "Pagination token from a previous call. Omit for the first page."
                },
                "page_size": {
                    "type": "integer",
                    "description": "Items per page, 1-500. Default 100."
                },
                "account": account_param(),
            },
            "required": ["app_token", "table_id"],
            "additionalProperties": false
        }),
    }
}

pub fn search_records_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_BITABLE_SEARCH_RECORDS.into(),
        description:
            "Structured query against a Feishu (Lark) bitable table. Supports field projection \
             (`field_names`), compound filter (Feishu filter object DSL), and sort. Returns the \
             same paginated shape as `feishu_bitable_list_records`. Required Feishu app scope: \
             `bitable:app.read` or `bitable:app`."
                .into(),
        tier: cfg(),
        internal: false,
        concurrent_safe: true,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "app_token": {"type": "string"},
                "table_id":  {"type": "string"},
                "view_id":   {"type": "string"},
                "field_names": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Restrict returned records to these field names. Omit to return all fields."
                },
                "filter": {
                    "type": "object",
                    "description": "Compound filter object (see Feishu bitable docs: `conjunction` + `conditions[]`)."
                },
                "sort": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "field_name": {"type": "string"},
                            "desc": {"type": "boolean"}
                        },
                        "required": ["field_name"]
                    },
                    "description": "Sort spec: array of `{field_name, desc?}`."
                },
                "automatic_fields": {
                    "type": "boolean",
                    "description": "If true, include the system-managed timestamp / user fields in the response."
                },
                "page_token": {"type": "string"},
                "page_size":  {"type": "integer"},
                "account": account_param(),
            },
            "required": ["app_token", "table_id"],
            "additionalProperties": false
        }),
    }
}

pub fn create_record_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_BITABLE_CREATE_RECORD.into(),
        description:
            "Insert a single record into a Feishu (Lark) bitable table. `fields` must match the \
             table's user-defined column schema (Field Name → value). Required Feishu app scope: \
             `bitable:app`."
                .into(),
        tier: cfg(),
        internal: false,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "app_token": {"type": "string"},
                "table_id":  {"type": "string"},
                "fields": {
                    "type": "object",
                    "description": "Field name → value map matching the table's column schema."
                },
                "account": account_param(),
            },
            "required": ["app_token", "table_id", "fields"],
            "additionalProperties": false
        }),
    }
}

pub fn list_views_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_BITABLE_LIST_VIEWS.into(),
        description:
            "List all views (grid / kanban / gantt / calendar / gallery / form) attached to a \
             Feishu (Lark) bitable table. Use the returned `view_id` with \
             `feishu_bitable_list_records` (`view_id` argument) or `feishu_bitable_get_view` \
             to drill into a specific view. Required Feishu app scope: `bitable:app.read` or \
             `bitable:app`."
                .into(),
        tier: cfg(),
        internal: false,
        concurrent_safe: true,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "app_token": {"type": "string"},
                "table_id":  {"type": "string"},
                "page_token": {"type": "string"},
                "page_size":  {"type": "integer", "description": "Items per page, default 100."},
                "account": account_param(),
            },
            "required": ["app_token", "table_id"],
            "additionalProperties": false
        }),
    }
}

pub fn get_view_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_BITABLE_GET_VIEW.into(),
        description:
            "Fetch a single Feishu (Lark) bitable view's full configuration — its `view_type`, \
             filter rules, sort spec, hidden field list, row height, etc. Useful when the agent \
             needs to understand how a view shapes its records before querying or when \
             explaining a view to the user. For just listing records under a view, use \
             `feishu_bitable_list_records` with `view_id`. Required Feishu app scope: \
             `bitable:app.read` or `bitable:app`."
                .into(),
        tier: cfg(),
        internal: false,
        concurrent_safe: true,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "app_token": {"type": "string"},
                "table_id":  {"type": "string"},
                "view_id":   {"type": "string"},
                "account": account_param(),
            },
            "required": ["app_token", "table_id", "view_id"],
            "additionalProperties": false
        }),
    }
}

pub fn list_dashboards_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_BITABLE_LIST_DASHBOARDS.into(),
        description:
            "List all dashboards (analytics / chart panels) attached to a Feishu (Lark) bitable \
             app. Returns dashboard IDs and names; the underlying chart definitions are not \
             exposed by this endpoint. Required Feishu app scope: `bitable:app.read` or \
             `bitable:app`."
                .into(),
        tier: cfg(),
        internal: false,
        concurrent_safe: true,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "app_token": {"type": "string"},
                "page_token": {"type": "string"},
                "page_size":  {"type": "integer", "description": "Items per page, default 100."},
                "account": account_param(),
            },
            "required": ["app_token"],
            "additionalProperties": false
        }),
    }
}

pub fn batch_update_records_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_BITABLE_BATCH_UPDATE_RECORDS.into(),
        description:
            "Update multiple records in a Feishu (Lark) bitable table in a single request \
             (max 1000). Each entry must include `record_id` plus the `fields` to merge — only \
             the listed fields are written; other fields are preserved. For full-row replacement, \
             delete and re-create. Required Feishu app scope: `bitable:app`."
                .into(),
        tier: cfg(),
        internal: false,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "app_token": {"type": "string"},
                "table_id":  {"type": "string"},
                "records": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "record_id": {"type": "string"},
                            "fields":    {"type": "object"}
                        },
                        "required": ["record_id", "fields"]
                    },
                    "description": "List of {record_id, fields} update entries. Up to 1000 per call."
                },
                "account": account_param(),
            },
            "required": ["app_token", "table_id", "records"],
            "additionalProperties": false
        }),
    }
}

fn get_required_records(args: &Value) -> Result<Vec<Value>> {
    let arr = args
        .get("records")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("`records` is required and must be an array"))?;
    if arr.is_empty() {
        return Err(anyhow!("`records` must contain at least one entry"));
    }
    if arr.len() > BITABLE_BATCH_UPDATE_MAX {
        return Err(anyhow!(
            "`records` has {} entries; Feishu's batch_update max is {}",
            arr.len(),
            BITABLE_BATCH_UPDATE_MAX
        ));
    }
    for (i, item) in arr.iter().enumerate() {
        let obj = item
            .as_object()
            .ok_or_else(|| anyhow!("`records[{}]` must be an object", i))?;
        if !obj.get("record_id").map(|v| v.is_string()).unwrap_or(false) {
            return Err(anyhow!(
                "`records[{}].record_id` is required and must be a string",
                i
            ));
        }
        if !obj.get("fields").map(|v| v.is_object()).unwrap_or(false) {
            return Err(anyhow!(
                "`records[{}].fields` is required and must be an object",
                i
            ));
        }
    }
    Ok(arr.clone())
}

// ── Execute fns ─────────────────────────────────────────────────

pub(crate) async fn execute_list_records(args: &Value) -> Result<String> {
    let app_token = arg_required_str(args, "app_token")?;
    let table_id = arg_required_str(args, "table_id")?;
    let view_id = arg_str(args, "view_id");
    let filter = arg_str(args, "filter");
    let page_token = arg_str(args, "page_token");
    let page_size = arg_u32(args, "page_size")?;
    let account = arg_str(args, "account");
    let api = resolve_feishu_api(account).await?;
    let page = api
        .bitable_list_records(crate::channel::feishu::api_bitable::BitableListRecordsReq {
            app_token,
            table_id,
            view_id,
            filter,
            page_token,
            page_size,
        })
        .await?;
    Ok(serde_json::to_string(&page)?)
}

pub(crate) async fn execute_search_records(args: &Value) -> Result<String> {
    let app_token = arg_required_str(args, "app_token")?;
    let table_id = arg_required_str(args, "table_id")?;
    let page_token = arg_str(args, "page_token");
    let page_size = arg_u32(args, "page_size")?;
    let account = arg_str(args, "account");

    // Build the search body from explicit fields so the LLM doesn't have
    // to know Feishu's exact request shape — only required fields go in.
    let mut body = serde_json::Map::new();
    if let Some(v) = arg_str(args, "view_id") {
        body.insert("view_id".into(), Value::String(v.into()));
    }
    if let Some(v) = args.get("field_names").filter(|v| v.is_array()) {
        body.insert("field_names".into(), v.clone());
    }
    if let Some(v) = args.get("filter").filter(|v| v.is_object()) {
        body.insert("filter".into(), v.clone());
    }
    if let Some(v) = args.get("sort").filter(|v| v.is_array()) {
        body.insert("sort".into(), v.clone());
    }
    if let Some(v) = args.get("automatic_fields").and_then(|v| v.as_bool()) {
        body.insert("automatic_fields".into(), Value::Bool(v));
    }

    let api = resolve_feishu_api(account).await?;
    let page = api
        .bitable_search_records(
            crate::channel::feishu::api_bitable::BitableSearchRecordsReq {
                app_token,
                table_id,
                body: Value::Object(body),
                page_token,
                page_size,
            },
        )
        .await?;
    Ok(serde_json::to_string(&page)?)
}

pub(crate) async fn execute_create_record(args: &Value) -> Result<String> {
    let app_token = arg_required_str(args, "app_token")?;
    let table_id = arg_required_str(args, "table_id")?;
    let fields = arg_required_object(args, "fields")?;
    let account = arg_str(args, "account");
    let api = resolve_feishu_api(account).await?;
    let rec = api
        .bitable_create_record(app_token, table_id, fields)
        .await?;
    Ok(serde_json::to_string(&rec)?)
}

pub(crate) async fn execute_batch_update_records(args: &Value) -> Result<String> {
    let app_token = arg_required_str(args, "app_token")?;
    let table_id = arg_required_str(args, "table_id")?;
    let records = get_required_records(args)?;
    let account = arg_str(args, "account");
    let api = resolve_feishu_api(account).await?;
    let result = api
        .bitable_batch_update_records(app_token, table_id, records)
        .await?;
    Ok(serde_json::to_string(&result)?)
}

pub(crate) async fn execute_list_views(args: &Value) -> Result<String> {
    let app_token = arg_required_str(args, "app_token")?;
    let table_id = arg_required_str(args, "table_id")?;
    let page_token = arg_str(args, "page_token");
    let page_size = arg_u32(args, "page_size")?;
    let account = arg_str(args, "account");
    let api = resolve_feishu_api(account).await?;
    let page = api
        .bitable_list_views(crate::channel::feishu::api_bitable::BitableListViewsReq {
            app_token,
            table_id,
            page_token,
            page_size,
        })
        .await?;
    Ok(serde_json::to_string(&page)?)
}

pub(crate) async fn execute_get_view(args: &Value) -> Result<String> {
    let app_token = arg_required_str(args, "app_token")?;
    let table_id = arg_required_str(args, "table_id")?;
    let view_id = arg_required_str(args, "view_id")?;
    let account = arg_str(args, "account");
    let api = resolve_feishu_api(account).await?;
    let view = api.bitable_get_view(app_token, table_id, view_id).await?;
    Ok(serde_json::to_string(&view)?)
}

pub(crate) async fn execute_list_dashboards(args: &Value) -> Result<String> {
    let app_token = arg_required_str(args, "app_token")?;
    let page_token = arg_str(args, "page_token");
    let page_size = arg_u32(args, "page_size")?;
    let account = arg_str(args, "account");
    let api = resolve_feishu_api(account).await?;
    let page = api
        .bitable_list_dashboards(app_token, page_token, page_size)
        .await?;
    Ok(serde_json::to_string(&page)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_have_expected_names() {
        assert_eq!(list_records_tool().name, TOOL_BITABLE_LIST_RECORDS);
        assert_eq!(search_records_tool().name, TOOL_BITABLE_SEARCH_RECORDS);
        assert_eq!(create_record_tool().name, TOOL_BITABLE_CREATE_RECORD);
        assert_eq!(
            batch_update_records_tool().name,
            TOOL_BITABLE_BATCH_UPDATE_RECORDS
        );
    }

    #[test]
    fn definitions_are_tier_configured_off_by_default() {
        for def in [
            list_records_tool(),
            search_records_tool(),
            create_record_tool(),
            batch_update_records_tool(),
        ] {
            match def.tier {
                ToolTier::Configured {
                    default_for_main,
                    default_for_others,
                    config_hint,
                    ..
                } => {
                    assert!(!default_for_main, "{}", def.name);
                    assert!(!default_for_others, "{}", def.name);
                    assert!(config_hint.contains("Feishu"));
                }
                _ => panic!("{} must be Tier 3 Configured", def.name),
            }
        }
    }

    #[tokio::test]
    async fn list_requires_app_token_and_table_id() {
        let err = execute_list_records(&json!({})).await.unwrap_err();
        assert!(err.to_string().contains("app_token"), "{}", err);

        let err = execute_list_records(&json!({"app_token": "a"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("table_id"), "{}", err);
    }

    #[tokio::test]
    async fn create_requires_fields_object() {
        let err = execute_create_record(&json!({
            "app_token": "a", "table_id": "t", "fields": "not-an-object"
        }))
        .await
        .unwrap_err();
        assert!(err.to_string().contains("fields"), "{}", err);
    }

    #[test]
    fn records_validation_empty_array_errors() {
        let err = get_required_records(&json!({"records": []})).unwrap_err();
        assert!(err.to_string().contains("at least one"), "{}", err);
    }

    #[test]
    fn records_validation_over_max_errors() {
        let arr: Vec<Value> = (0..(BITABLE_BATCH_UPDATE_MAX + 1))
            .map(|i| json!({"record_id": format!("r{}", i), "fields": {}}))
            .collect();
        let err = get_required_records(&json!({"records": arr})).unwrap_err();
        assert!(err.to_string().contains("max"), "{}", err);
    }

    #[test]
    fn records_validation_missing_record_id_errors() {
        let err = get_required_records(&json!({"records": [{"fields": {}}]})).unwrap_err();
        assert!(err.to_string().contains("record_id"), "{}", err);
    }

    #[test]
    fn records_validation_fields_not_object_errors() {
        let err = get_required_records(&json!({
            "records": [{"record_id": "r1", "fields": "x"}]
        }))
        .unwrap_err();
        assert!(err.to_string().contains("fields"), "{}", err);
    }
}
