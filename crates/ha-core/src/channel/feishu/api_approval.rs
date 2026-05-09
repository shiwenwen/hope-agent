//! approval (审批 / Lark Approval) REST methods.
//!
//! C6 of the v0.2.0 Feishu roadmap. Five endpoints:
//! - `approval_create_instance` — submit a new approval instance for an
//!   existing approval definition (HIGH risk: creates a real approval
//!   that travels to the configured approvers' inbox).
//! - `approval_get_instance` — fetch an instance's status, form fields,
//!   and timeline.
//! - `approval_cancel_instance` — withdraw an instance the bot created.
//! - `approval_list_instances` — paginated list of instance codes by
//!   approval definition + time range + status filter.
//! - `approval_subscribe` — register the bot to receive event push for an
//!   approval definition (events flow to the existing channel WebSocket
//!   gateway; v0.2.0 only logs them — see Phase B.2 for behavior).
//!
//! Reference: <https://open.feishu.cn/document/server-docs/approval-v4/instance/create>

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::api::FeishuApi;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApprovalCreateResult {
    pub instance_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApprovalInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_code: Option<String>,
    pub instance_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<String>,
    /// Form fields as user-defined JSON (per-approval schema).
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub form: Value,
    /// Approval timeline (tasks + comments + transitions); shape varies by
    /// approval definition — pass through as JSON.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub timeline: Value,
    /// Tasks (one per approver step).
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub task_list: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApprovalInstanceList {
    #[serde(default)]
    pub instance_code_list: Vec<String>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_token: Option<String>,
}

impl FeishuApi {
    /// `POST /open-apis/approval/v4/instances` — create a new approval
    /// instance. `form` must match the approval definition's field schema
    /// (typically a JSON-encoded array of `{id, type, value}`).
    pub async fn approval_create_instance(
        &self,
        approval_code: &str,
        user_id: &str,
        form: &str,
    ) -> Result<ApprovalCreateResult> {
        let url = format!("{}/open-apis/approval/v4/instances", self.base_url());
        let body = serde_json::json!({
            "approval_code": approval_code,
            "user_id": user_id,
            "form": form,
        });
        let resp = self
            .authorized_request(reqwest::Method::POST, &url)
            .await?
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to POST approval_create_instance: {}", e))?;
        Ok(self
            .parse_envelope::<ApprovalCreateResult>(resp, "approval_create_instance")
            .await?
            .unwrap_or_default())
    }

    /// `GET /open-apis/approval/v4/instances/{instance_code}` — fetch
    /// instance detail (status, form, timeline, tasks).
    pub async fn approval_get_instance(&self, instance_code: &str) -> Result<ApprovalInstance> {
        let url = format!(
            "{}/open-apis/approval/v4/instances/{}",
            self.base_url(),
            instance_code
        );
        let resp = self
            .authorized_request(reqwest::Method::GET, &url)
            .await?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to GET approval_get_instance: {}", e))?;
        Ok(self
            .parse_envelope::<ApprovalInstance>(resp, "approval_get_instance")
            .await?
            .unwrap_or_default())
    }

    /// `POST /open-apis/approval/v4/instances/cancel` — withdraw an
    /// instance. Only the original submitter (or an admin) can cancel.
    pub async fn approval_cancel_instance(
        &self,
        approval_code: &str,
        instance_code: &str,
        user_id: &str,
    ) -> Result<()> {
        let url = format!("{}/open-apis/approval/v4/instances/cancel", self.base_url());
        let body = serde_json::json!({
            "approval_code": approval_code,
            "instance_code": instance_code,
            "user_id": user_id,
        });
        let resp = self
            .authorized_request(reqwest::Method::POST, &url)
            .await?
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to POST approval_cancel_instance: {}", e))?;
        let _: Option<Value> = self
            .parse_envelope(resp, "approval_cancel_instance")
            .await?;
        Ok(())
    }

    /// `GET /open-apis/approval/v4/instances` — paginated list of instance
    /// codes by approval definition. Optional `start_time` / `end_time` are
    /// epoch-millisecond strings.
    pub async fn approval_list_instances(
        &self,
        approval_code: &str,
        start_time: Option<&str>,
        end_time: Option<&str>,
        page_token: Option<&str>,
        page_size: Option<u32>,
    ) -> Result<ApprovalInstanceList> {
        let mut url = format!("{}/open-apis/approval/v4/instances", self.base_url());
        let mut params: Vec<(&str, String)> =
            vec![("approval_code", approval_code.to_string())];
        if let Some(v) = start_time {
            params.push(("start_time", v.to_string()));
        }
        if let Some(v) = end_time {
            params.push(("end_time", v.to_string()));
        }
        if let Some(v) = page_token {
            params.push(("page_token", v.to_string()));
        }
        if let Some(v) = page_size {
            params.push(("page_size", v.to_string()));
        }
        let qs = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        url.push('?');
        url.push_str(&qs);
        let resp = self
            .authorized_request(reqwest::Method::GET, &url)
            .await?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to GET approval_list_instances: {}", e))?;
        Ok(self
            .parse_envelope::<ApprovalInstanceList>(resp, "approval_list_instances")
            .await?
            .unwrap_or_default())
    }

    /// `POST /open-apis/approval/v4/approvals/{approval_code}/subscribe` —
    /// enable event push for an approval definition. v0.2.0 logs received
    /// events at the dispatcher; full business behavior (e.g. inject
    /// approval-status changes back into a session) is deferred.
    pub async fn approval_subscribe(&self, approval_code: &str) -> Result<()> {
        let url = format!(
            "{}/open-apis/approval/v4/approvals/{}/subscribe",
            self.base_url(),
            approval_code
        );
        let resp = self
            .authorized_request(reqwest::Method::POST, &url)
            .await?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to POST approval_subscribe: {}", e))?;
        let _: Option<Value> = self.parse_envelope(resp, "approval_subscribe").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::feishu::auth::FeishuAuth;
    use std::sync::Arc;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn mock_api(server: &MockServer) -> FeishuApi {
        Mock::given(method("POST"))
            .and(path("/open-apis/auth/v3/tenant_access_token/internal/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0, "msg": "ok",
                "tenant_access_token": "t", "expire": 7200
            })))
            .mount(server)
            .await;
        FeishuApi::new(Arc::new(FeishuAuth::new("c", "s", &server.uri())))
    }

    #[tokio::test]
    async fn create_instance_returns_code() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        Mock::given(method("POST"))
            .and(path("/open-apis/approval/v4/instances"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0, "msg": "ok",
                "data": {"instance_code": "INST-001"}
            })))
            .mount(&server)
            .await;
        let r = api
            .approval_create_instance("APP-1", "ou_user", "[]")
            .await
            .unwrap();
        assert_eq!(r.instance_code, "INST-001");
    }

    #[tokio::test]
    async fn get_instance_returns_status_and_form() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        Mock::given(method("GET"))
            .and(path("/open-apis/approval/v4/instances/INST-001"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0, "msg": "ok",
                "data": {"instance_code": "INST-001", "status": "PENDING", "form": "[]"}
            })))
            .mount(&server)
            .await;
        let inst = api.approval_get_instance("INST-001").await.unwrap();
        assert_eq!(inst.instance_code, "INST-001");
        assert_eq!(inst.status.as_deref(), Some("PENDING"));
    }

    #[tokio::test]
    async fn cancel_instance_returns_ok() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        Mock::given(method("POST"))
            .and(path("/open-apis/approval/v4/instances/cancel"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0, "msg": "ok"
            })))
            .mount(&server)
            .await;
        api.approval_cancel_instance("APP-1", "INST-001", "ou_user")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn list_instances_returns_codes() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        Mock::given(method("GET"))
            .and(path("/open-apis/approval/v4/instances"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0, "msg": "ok",
                "data": {"instance_code_list": ["I1", "I2"], "has_more": false}
            })))
            .mount(&server)
            .await;
        let list = api
            .approval_list_instances("APP-1", None, None, None, Some(10))
            .await
            .unwrap();
        assert_eq!(list.instance_code_list, vec!["I1", "I2"]);
    }

    #[tokio::test]
    async fn create_propagates_envelope_error() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        Mock::given(method("POST"))
            .and(path("/open-apis/approval/v4/instances"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 99991672, "msg": "no scope"
            })))
            .mount(&server)
            .await;
        let err = api
            .approval_create_instance("A", "u", "[]")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("99991672"), "{}", err);
    }
}
