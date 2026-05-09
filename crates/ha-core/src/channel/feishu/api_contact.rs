//! contact (联系人 / Lark Contact) REST methods.
//!
//! Sensitive: returns employee
//! personal info (name / email / mobile / department / job_title /
//! avatar). Tools must surface this in the description so the agent
//! handles it carefully (no echoing in untrusted contexts, no
//! cross-tenant leakage).
//!
//! Reference: <https://open.feishu.cn/document/server-docs/contact-v3/user/get>

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::api::FeishuApi;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContactUser {
    /// Pass-through of Feishu's user object — names / IDs / mobile / email / etc.
    #[serde(default, flatten)]
    pub fields: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct UserData {
    user: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContactBatchUsersResult {
    #[serde(default)]
    pub items: Vec<Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct DepartmentData {
    department: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContactSearchResult {
    #[serde(default)]
    pub items: Vec<Value>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_token: Option<String>,
}

impl FeishuApi {
    /// `GET /open-apis/contact/v3/users/{user_id}` — fetch one user.
    /// `user_id_type` ∈ `open_id` / `union_id` / `user_id`.
    pub async fn contact_get_user(&self, user_id: &str, user_id_type: Option<&str>) -> Result<Value> {
        let mut url = format!(
            "{}/open-apis/contact/v3/users/{}",
            self.base_url(),
            user_id
        );
        let params: Vec<(&str, String)> = user_id_type
            .map(|t| vec![("user_id_type", t.to_string())])
            .unwrap_or_default();
        super::api::append_query(&mut url, &params);
        let resp = self
            .authorized_request(reqwest::Method::GET, &url)
            .await?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to GET contact_get_user: {}", e))?;
        let data: UserData = self
            .parse_envelope(resp, "contact_get_user")
            .await?
            .ok_or_else(|| anyhow!("contact_get_user response missing 'data'"))?;
        Ok(data.user)
    }

    /// `POST /open-apis/contact/v3/users/batch` — fetch up to 50 users at once.
    pub async fn contact_batch_get_users(
        &self,
        user_ids: Vec<String>,
        user_id_type: Option<&str>,
    ) -> Result<ContactBatchUsersResult> {
        if user_ids.is_empty() {
            return Err(anyhow!("contact_batch_get_users: user_ids is empty"));
        }
        if user_ids.len() > 50 {
            return Err(anyhow!(
                "contact_batch_get_users: {} ids exceeds Feishu's max of 50",
                user_ids.len()
            ));
        }
        let mut url = format!("{}/open-apis/contact/v3/users/batch", self.base_url());
        let params: Vec<(&str, String)> = user_id_type
            .map(|t| vec![("user_id_type", t.to_string())])
            .unwrap_or_default();
        super::api::append_query(&mut url, &params);
        let body = serde_json::json!({"user_ids": user_ids});
        let resp = self
            .authorized_request(reqwest::Method::POST, &url)
            .await?
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to POST contact_batch_get_users: {}", e))?;
        Ok(self
            .parse_envelope::<ContactBatchUsersResult>(resp, "contact_batch_get_users")
            .await?
            .unwrap_or_default())
    }

    /// `GET /open-apis/contact/v3/departments/{department_id}` — fetch department detail.
    pub async fn contact_get_department(
        &self,
        department_id: &str,
        department_id_type: Option<&str>,
    ) -> Result<Value> {
        let mut url = format!(
            "{}/open-apis/contact/v3/departments/{}",
            self.base_url(),
            department_id
        );
        let params: Vec<(&str, String)> = department_id_type
            .map(|t| vec![("department_id_type", t.to_string())])
            .unwrap_or_default();
        super::api::append_query(&mut url, &params);
        let resp = self
            .authorized_request(reqwest::Method::GET, &url)
            .await?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to GET contact_get_department: {}", e))?;
        let data: DepartmentData = self
            .parse_envelope(resp, "contact_get_department")
            .await?
            .ok_or_else(|| anyhow!("contact_get_department response missing 'data'"))?;
        Ok(data.department)
    }

    /// `GET /open-apis/contact/v3/users/find_by_department` — list users in a department.
    pub async fn contact_search_users_by_department(
        &self,
        department_id: &str,
        page_token: Option<&str>,
        page_size: Option<u32>,
    ) -> Result<ContactSearchResult> {
        let mut url = format!(
            "{}/open-apis/contact/v3/users/find_by_department",
            self.base_url()
        );
        let mut params: Vec<(&str, String)> =
            vec![("department_id", department_id.to_string())];
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
            .map_err(|e| anyhow!("Failed to GET contact_search_users_by_department: {}", e))?;
        Ok(self
            .parse_envelope::<ContactSearchResult>(resp, "contact_search_users_by_department")
            .await?
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use super::super::api::test_support::mock_api;

    #[tokio::test]
    async fn get_user_returns_user_object() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        Mock::given(method("GET"))
            .and(path("/open-apis/contact/v3/users/ou_user1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0, "msg": "ok",
                "data": {"user": {"open_id": "ou_user1", "name": "张三"}}
            })))
            .mount(&server)
            .await;
        let user = api.contact_get_user("ou_user1", None).await.unwrap();
        assert_eq!(user["name"], "张三");
    }

    #[tokio::test]
    async fn batch_users_rejects_empty() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        let err = api
            .contact_batch_get_users(Vec::new(), None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("empty"), "{}", err);
    }

    #[tokio::test]
    async fn batch_users_rejects_over_50() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        let many: Vec<String> = (0..51).map(|i| format!("ou_{}", i)).collect();
        let err = api
            .contact_batch_get_users(many, None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("max"), "{}", err);
    }

    #[tokio::test]
    async fn get_department_returns_object() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        Mock::given(method("GET"))
            .and(path("/open-apis/contact/v3/departments/od_d1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0, "msg": "ok",
                "data": {"department": {"name": "engineering", "department_id": "od_d1"}}
            })))
            .mount(&server)
            .await;
        let d = api.contact_get_department("od_d1", None).await.unwrap();
        assert_eq!(d["name"], "engineering");
    }
}
