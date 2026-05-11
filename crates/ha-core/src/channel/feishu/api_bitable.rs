//! bitable (多维表格 / Lark Base) REST methods.
//!
//! Extends [`FeishuApi`] with the four core bitable record endpoints used
//! by the C2 tools (list / search / create / batch_update). All responses
//! pass through the parent module's `parse_envelope`; record `fields` is
//! left as `serde_json::Value` because every bitable table has a different
//! user-defined column schema.
//!
//! References:
//! - <https://open.feishu.cn/document/server-docs/docs/bitable-v1/app-table-record/list>
//! - <https://open.feishu.cn/document/server-docs/docs/bitable-v1/app-table-record/search>
//! - <https://open.feishu.cn/document/server-docs/docs/bitable-v1/app-table-record/create>
//! - <https://open.feishu.cn/document/server-docs/docs/bitable-v1/app-table-record/batch_update>

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::api::FeishuApi;

/// Hard cap from Feishu's batch_update endpoint.
pub const BITABLE_BATCH_UPDATE_MAX: usize = 1000;

// ── Response types ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BitableRecord {
    /// Returned in list / search / create responses; absent on update payload input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
    /// Field name → value map; per-table user-defined schema.
    #[serde(default)]
    pub fields: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_time: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified_time: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified_by: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BitableRecordsPage {
    #[serde(default)]
    pub items: Vec<BitableRecord>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BitableCreateRecordResult {
    #[serde(default)]
    pub record: BitableRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BitableBatchUpdateResult {
    #[serde(default)]
    pub records: Vec<BitableRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BitableView {
    pub view_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_name: Option<String>,
    /// `grid` / `kanban` / `gantt` / `calendar` / `gallery` / `form` / etc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_type: Option<String>,
    /// View-specific configuration (filter / sort / hidden_fields / row_height
    /// / etc.). Shape depends on `view_type`; pass through as JSON.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub property: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BitableViewsPage {
    #[serde(default)]
    pub items: Vec<BitableView>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct BitableViewData {
    view: BitableView,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BitableDashboard {
    pub dashboard_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dashboard_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BitableDashboardsPage {
    #[serde(default)]
    pub dashboards: Vec<BitableDashboard>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

// ── Request structs (named-field bag style) ─────────────────────
//
// list / search / list_views all take (app_token, table_id) + a long
// optional pagination tail; passing 4-6 positional `Option<&str>` is easy
// to mis-order. These structs make every field explicit at the call site.

/// Args for [`FeishuApi::bitable_list_records`].
pub struct BitableListRecordsReq<'a> {
    pub app_token: &'a str,
    pub table_id: &'a str,
    pub view_id: Option<&'a str>,
    pub filter: Option<&'a str>,
    pub page_token: Option<&'a str>,
    pub page_size: Option<u32>,
}

/// Args for [`FeishuApi::bitable_search_records`].
pub struct BitableSearchRecordsReq<'a> {
    pub app_token: &'a str,
    pub table_id: &'a str,
    pub body: Value,
    pub page_token: Option<&'a str>,
    pub page_size: Option<u32>,
}

/// Args for [`FeishuApi::bitable_list_views`].
pub struct BitableListViewsReq<'a> {
    pub app_token: &'a str,
    pub table_id: &'a str,
    pub page_token: Option<&'a str>,
    pub page_size: Option<u32>,
}

// ── Public methods on FeishuApi ─────────────────────────────────

impl FeishuApi {
    /// `GET /open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records`
    /// — paginated list with optional view + filter expression.
    pub async fn bitable_list_records(
        &self,
        req: BitableListRecordsReq<'_>,
    ) -> Result<BitableRecordsPage> {
        let mut url = format!(
            "{}/open-apis/bitable/v1/apps/{}/tables/{}/records",
            self.base_url(),
            req.app_token,
            req.table_id
        );
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(v) = req.view_id {
            params.push(("view_id", v.to_string()));
        }
        if let Some(f) = req.filter {
            params.push(("filter", f.to_string()));
        }
        if let Some(t) = req.page_token {
            params.push(("page_token", t.to_string()));
        }
        if let Some(s) = req.page_size {
            params.push(("page_size", s.to_string()));
        }
        super::api::append_query(&mut url, &params);
        let resp = self
            .authorized_request(reqwest::Method::GET, &url)
            .await?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to GET bitable_list_records: {}", e))?;
        Ok(self
            .parse_envelope::<BitableRecordsPage>(resp, "bitable_list_records")
            .await?
            .unwrap_or_default())
    }

    /// `POST /open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records/search`
    /// — structured query with field projection / filter / sort.
    /// `body` is the request payload (passed through unchanged so callers
    /// can use the full Feishu filter DSL); `page_token` and `page_size`
    /// go on the URL as query params.
    pub async fn bitable_search_records(
        &self,
        req: BitableSearchRecordsReq<'_>,
    ) -> Result<BitableRecordsPage> {
        let mut url = format!(
            "{}/open-apis/bitable/v1/apps/{}/tables/{}/records/search",
            self.base_url(),
            req.app_token,
            req.table_id
        );
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(t) = req.page_token {
            params.push(("page_token", t.to_string()));
        }
        if let Some(s) = req.page_size {
            params.push(("page_size", s.to_string()));
        }
        super::api::append_query(&mut url, &params);
        let resp = self
            .authorized_request(reqwest::Method::POST, &url)
            .await?
            .json(&req.body)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to POST bitable_search_records: {}", e))?;
        Ok(self
            .parse_envelope::<BitableRecordsPage>(resp, "bitable_search_records")
            .await?
            .unwrap_or_default())
    }

    /// `POST /open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records`
    /// — create a single record. `fields` is the user-defined field name → value map.
    pub async fn bitable_create_record(
        &self,
        app_token: &str,
        table_id: &str,
        fields: Value,
    ) -> Result<BitableRecord> {
        let url = format!(
            "{}/open-apis/bitable/v1/apps/{}/tables/{}/records",
            self.base_url(),
            app_token,
            table_id
        );
        let body = serde_json::json!({"fields": fields});
        let resp = self
            .authorized_request(reqwest::Method::POST, &url)
            .await?
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to POST bitable_create_record: {}", e))?;
        let data: BitableCreateRecordResult = self
            .parse_envelope(resp, "bitable_create_record")
            .await?
            .unwrap_or_default();
        Ok(data.record)
    }

    /// `POST /open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records/batch_update`
    /// — update up to [`BITABLE_BATCH_UPDATE_MAX`] records in one request.
    /// Each `records[i]` must include `record_id` + the `fields` to merge.
    pub async fn bitable_batch_update_records(
        &self,
        app_token: &str,
        table_id: &str,
        records: Vec<Value>,
    ) -> Result<BitableBatchUpdateResult> {
        if records.is_empty() {
            return Err(anyhow!(
                "bitable_batch_update_records: `records` is empty (must contain 1..={})",
                BITABLE_BATCH_UPDATE_MAX
            ));
        }
        if records.len() > BITABLE_BATCH_UPDATE_MAX {
            return Err(anyhow!(
                "bitable_batch_update_records: {} records exceeds Feishu's max of {}",
                records.len(),
                BITABLE_BATCH_UPDATE_MAX
            ));
        }
        let url = format!(
            "{}/open-apis/bitable/v1/apps/{}/tables/{}/records/batch_update",
            self.base_url(),
            app_token,
            table_id
        );
        let body = serde_json::json!({"records": records});
        let resp = self
            .authorized_request(reqwest::Method::POST, &url)
            .await?
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to POST bitable_batch_update_records: {}", e))?;
        Ok(self
            .parse_envelope::<BitableBatchUpdateResult>(resp, "bitable_batch_update_records")
            .await?
            .unwrap_or_default())
    }

    /// `GET /open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/views`
    /// — paginated list of views in a table.
    pub async fn bitable_list_views(
        &self,
        req: BitableListViewsReq<'_>,
    ) -> Result<BitableViewsPage> {
        let mut url = format!(
            "{}/open-apis/bitable/v1/apps/{}/tables/{}/views",
            self.base_url(),
            req.app_token,
            req.table_id
        );
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(t) = req.page_token {
            params.push(("page_token", t.to_string()));
        }
        if let Some(s) = req.page_size {
            params.push(("page_size", s.to_string()));
        }
        super::api::append_query(&mut url, &params);
        let resp = self
            .authorized_request(reqwest::Method::GET, &url)
            .await?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to GET bitable_list_views: {}", e))?;
        Ok(self
            .parse_envelope::<BitableViewsPage>(resp, "bitable_list_views")
            .await?
            .unwrap_or_default())
    }

    /// `GET /open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/views/{view_id}`
    /// — fetch a single view's full configuration (filter, sort, hidden
    /// fields, row height, etc.). Useful when the agent needs to understand
    /// how a view shapes its records before querying.
    pub async fn bitable_get_view(
        &self,
        app_token: &str,
        table_id: &str,
        view_id: &str,
    ) -> Result<BitableView> {
        let url = format!(
            "{}/open-apis/bitable/v1/apps/{}/tables/{}/views/{}",
            self.base_url(),
            app_token,
            table_id,
            view_id
        );
        let resp = self
            .authorized_request(reqwest::Method::GET, &url)
            .await?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to GET bitable_get_view: {}", e))?;
        let data: BitableViewData = self
            .parse_envelope(resp, "bitable_get_view")
            .await?
            .ok_or_else(|| anyhow!("bitable_get_view response missing 'data'"))?;
        Ok(data.view)
    }

    /// `GET /open-apis/bitable/v1/apps/{app_token}/dashboards` — list
    /// dashboards (analytics views) attached to a bitable app.
    pub async fn bitable_list_dashboards(
        &self,
        app_token: &str,
        page_token: Option<&str>,
        page_size: Option<u32>,
    ) -> Result<BitableDashboardsPage> {
        let mut url = format!(
            "{}/open-apis/bitable/v1/apps/{}/dashboards",
            self.base_url(),
            app_token
        );
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(t) = page_token {
            params.push(("page_token", t.to_string()));
        }
        if let Some(s) = page_size {
            params.push(("page_size", s.to_string()));
        }
        super::api::append_query(&mut url, &params);
        let resp = self
            .authorized_request(reqwest::Method::GET, &url)
            .await?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to GET bitable_list_dashboards: {}", e))?;
        Ok(self
            .parse_envelope::<BitableDashboardsPage>(resp, "bitable_list_dashboards")
            .await?
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::super::api::test_support::mock_api;
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn list_records_passes_view_filter_pagination() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;

        Mock::given(method("GET"))
            .and(path(
                "/open-apis/bitable/v1/apps/bascnAbc/tables/tblXyz/records",
            ))
            .and(query_param("view_id", "vewQ"))
            .and(query_param("filter", "CurrentValue.[Status]=\"Done\""))
            .and(query_param("page_size", "50"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "success",
                "data": {
                    "items": [
                        {"record_id": "rec1", "fields": {"Name": "x"}}
                    ],
                    "has_more": false,
                    "total": 1
                }
            })))
            .mount(&server)
            .await;

        let page = api
            .bitable_list_records(BitableListRecordsReq {
                app_token: "bascnAbc",
                table_id: "tblXyz",
                view_id: Some("vewQ"),
                filter: Some("CurrentValue.[Status]=\"Done\""),
                page_token: None,
                page_size: Some(50),
            })
            .await
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].record_id.as_deref(), Some("rec1"));
        assert_eq!(page.total, Some(1));
        assert!(!page.has_more);
    }

    #[tokio::test]
    async fn search_records_posts_body_and_query_pagination() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;

        Mock::given(method("POST"))
            .and(path(
                "/open-apis/bitable/v1/apps/bascnAbc/tables/tblXyz/records/search",
            ))
            .and(query_param("page_size", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "success",
                "data": {
                    "items": [],
                    "has_more": true,
                    "page_token": "next-tok"
                }
            })))
            .mount(&server)
            .await;

        let body = serde_json::json!({
            "field_names": ["Name", "Status"],
            "sort": [{"field_name": "CreatedAt", "desc": true}]
        });
        let page = api
            .bitable_search_records(BitableSearchRecordsReq {
                app_token: "bascnAbc",
                table_id: "tblXyz",
                body,
                page_token: None,
                page_size: Some(100),
            })
            .await
            .unwrap();
        assert!(page.has_more);
        assert_eq!(page.page_token.as_deref(), Some("next-tok"));
    }

    #[tokio::test]
    async fn create_record_returns_generated_record_id() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;

        Mock::given(method("POST"))
            .and(path(
                "/open-apis/bitable/v1/apps/bascnAbc/tables/tblXyz/records",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "success",
                "data": {
                    "record": {
                        "record_id": "recNew",
                        "fields": {"Name": "Hello"}
                    }
                }
            })))
            .mount(&server)
            .await;

        let rec = api
            .bitable_create_record("bascnAbc", "tblXyz", serde_json::json!({"Name": "Hello"}))
            .await
            .unwrap();
        assert_eq!(rec.record_id.as_deref(), Some("recNew"));
    }

    #[tokio::test]
    async fn batch_update_returns_updated_records() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;

        Mock::given(method("POST"))
            .and(path(
                "/open-apis/bitable/v1/apps/bascnAbc/tables/tblXyz/records/batch_update",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "success",
                "data": {
                    "records": [
                        {"record_id": "rec1", "fields": {"Status": "Done"}}
                    ]
                }
            })))
            .mount(&server)
            .await;

        let result = api
            .bitable_batch_update_records(
                "bascnAbc",
                "tblXyz",
                vec![serde_json::json!({
                    "record_id": "rec1",
                    "fields": {"Status": "Done"}
                })],
            )
            .await
            .unwrap();
        assert_eq!(result.records.len(), 1);
        assert_eq!(result.records[0].record_id.as_deref(), Some("rec1"));
    }

    #[tokio::test]
    async fn batch_update_rejects_empty_input() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        let err = api
            .bitable_batch_update_records("bascnAbc", "tblXyz", Vec::new())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("empty"), "{}", err);
    }

    #[tokio::test]
    async fn batch_update_rejects_over_max() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        let too_many: Vec<Value> = (0..(BITABLE_BATCH_UPDATE_MAX + 1))
            .map(|i| serde_json::json!({"record_id": format!("r{}", i), "fields": {}}))
            .collect();
        let err = api
            .bitable_batch_update_records("bascnAbc", "tblXyz", too_many)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("exceeds"), "{}", err);
    }

    #[tokio::test]
    async fn list_records_propagates_envelope_error() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        Mock::given(method("GET"))
            .and(path(
                "/open-apis/bitable/v1/apps/bascnAbc/tables/tblXyz/records",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 99991663,
                "msg": "table not found"
            })))
            .mount(&server)
            .await;
        let err = api
            .bitable_list_records(BitableListRecordsReq {
                app_token: "bascnAbc",
                table_id: "tblXyz",
                view_id: None,
                filter: None,
                page_token: None,
                page_size: None,
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("99991663"), "{}", err);
    }

    #[tokio::test]
    async fn list_views_returns_view_array() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        Mock::given(method("GET"))
            .and(path(
                "/open-apis/bitable/v1/apps/bascnAbc/tables/tblXyz/views",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "success",
                "data": {
                    "items": [
                        {"view_id": "vewA", "view_name": "Grid", "view_type": "grid"},
                        {"view_id": "vewB", "view_name": "Calendar", "view_type": "calendar"}
                    ],
                    "has_more": false
                }
            })))
            .mount(&server)
            .await;
        let page = api
            .bitable_list_views(BitableListViewsReq {
                app_token: "bascnAbc",
                table_id: "tblXyz",
                page_token: None,
                page_size: None,
            })
            .await
            .unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].view_id, "vewA");
        assert_eq!(page.items[1].view_type.as_deref(), Some("calendar"));
    }

    #[tokio::test]
    async fn get_view_returns_full_view_with_property() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        Mock::given(method("GET"))
            .and(path(
                "/open-apis/bitable/v1/apps/bascnAbc/tables/tblXyz/views/vewA",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "success",
                "data": {
                    "view": {
                        "view_id": "vewA",
                        "view_name": "Done items",
                        "view_type": "grid",
                        "property": {
                            "filter_info": {"conjunction": "and", "conditions": []},
                            "hidden_fields": ["Notes"]
                        }
                    }
                }
            })))
            .mount(&server)
            .await;
        let view = api
            .bitable_get_view("bascnAbc", "tblXyz", "vewA")
            .await
            .unwrap();
        assert_eq!(view.view_id, "vewA");
        assert_eq!(view.view_name.as_deref(), Some("Done items"));
        assert!(view.property.is_object());
        assert!(view.property.get("hidden_fields").is_some());
    }

    #[tokio::test]
    async fn list_dashboards_returns_array() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        Mock::given(method("GET"))
            .and(path("/open-apis/bitable/v1/apps/bascnAbc/dashboards"))
            .and(query_param("page_size", "10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "success",
                "data": {
                    "dashboards": [
                        {"dashboard_id": "dashA", "dashboard_name": "OKR overview"}
                    ],
                    "has_more": false
                }
            })))
            .mount(&server)
            .await;
        let page = api
            .bitable_list_dashboards("bascnAbc", None, Some(10))
            .await
            .unwrap();
        assert_eq!(page.dashboards.len(), 1);
        assert_eq!(page.dashboards[0].dashboard_id, "dashA");
    }
}
