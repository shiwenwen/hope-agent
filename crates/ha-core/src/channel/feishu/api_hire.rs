//! hire (招聘 / Lark Hire) REST methods.
//!
//! Note: not all tenants subscribe to
//! the hire module; calls return code `99991663` (param) or
//! `1061004` (module not enabled) when unavailable. Surface this in the
//! tool descriptions.
//!
//! Reference: <https://open.feishu.cn/document/server-docs/hire-v1/job/list>

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::api::FeishuApi;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HireListResult {
    #[serde(default)]
    pub items: Vec<Value>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct JobData {
    job: Value,
}
#[derive(Debug, Clone, Deserialize)]
struct TalentData {
    talent: Value,
}

impl FeishuApi {
    pub async fn hire_list_jobs(
        &self,
        page_token: Option<&str>,
        page_size: Option<u32>,
    ) -> Result<HireListResult> {
        list_endpoint(self, "/open-apis/hire/v1/jobs", page_token, page_size, "hire_list_jobs").await
    }

    pub async fn hire_get_job(&self, job_id: &str) -> Result<Value> {
        let url = format!("{}/open-apis/hire/v1/jobs/{}", self.base_url(), job_id);
        let resp = self
            .authorized_request(reqwest::Method::GET, &url)
            .await?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to GET hire_get_job: {}", e))?;
        let data: JobData = self
            .parse_envelope(resp, "hire_get_job")
            .await?
            .ok_or_else(|| anyhow!("hire_get_job response missing 'data'"))?;
        Ok(data.job)
    }

    pub async fn hire_list_talents(
        &self,
        page_token: Option<&str>,
        page_size: Option<u32>,
    ) -> Result<HireListResult> {
        list_endpoint(
            self,
            "/open-apis/hire/v1/talents",
            page_token,
            page_size,
            "hire_list_talents",
        )
        .await
    }

    pub async fn hire_get_talent(&self, talent_id: &str) -> Result<Value> {
        let url = format!(
            "{}/open-apis/hire/v1/talents/{}",
            self.base_url(),
            talent_id
        );
        let resp = self
            .authorized_request(reqwest::Method::GET, &url)
            .await?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to GET hire_get_talent: {}", e))?;
        let data: TalentData = self
            .parse_envelope(resp, "hire_get_talent")
            .await?
            .ok_or_else(|| anyhow!("hire_get_talent response missing 'data'"))?;
        Ok(data.talent)
    }

    pub async fn hire_list_applications(
        &self,
        page_token: Option<&str>,
        page_size: Option<u32>,
    ) -> Result<HireListResult> {
        list_endpoint(
            self,
            "/open-apis/hire/v1/applications",
            page_token,
            page_size,
            "hire_list_applications",
        )
        .await
    }
}

async fn list_endpoint(
    api: &FeishuApi,
    path_str: &str,
    page_token: Option<&str>,
    page_size: Option<u32>,
    label: &str,
) -> Result<HireListResult> {
    let mut url = format!("{}{}", api.base_url(), path_str);
    let mut params: Vec<(&str, String)> = Vec::new();
    if let Some(t) = page_token {
        params.push(("page_token", t.to_string()));
    }
    if let Some(s) = page_size {
        params.push(("page_size", s.to_string()));
    }
    super::api::append_query(&mut url, &params);
    let resp = api
        .authorized_request(reqwest::Method::GET, &url)
        .await?
        .send()
        .await
        .map_err(|e| anyhow!("Failed to GET {}: {}", label, e))?;
    Ok(api
        .parse_envelope::<HireListResult>(resp, label)
        .await?
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use super::super::api::test_support::mock_api;

    #[tokio::test]
    async fn list_jobs_returns_items() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        Mock::given(method("GET"))
            .and(path("/open-apis/hire/v1/jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0, "msg": "ok",
                "data": {"items": [{"id": "job1"}], "has_more": false}
            })))
            .mount(&server)
            .await;
        let r = api.hire_list_jobs(None, Some(10)).await.unwrap();
        assert_eq!(r.items.len(), 1);
    }

    #[tokio::test]
    async fn get_job_returns_object() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        Mock::given(method("GET"))
            .and(path("/open-apis/hire/v1/jobs/job1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0, "msg": "ok",
                "data": {"job": {"id": "job1", "title": "SWE"}}
            })))
            .mount(&server)
            .await;
        let r = api.hire_get_job("job1").await.unwrap();
        assert_eq!(r["title"], "SWE");
    }

    #[tokio::test]
    async fn module_not_enabled_propagates() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        Mock::given(method("GET"))
            .and(path("/open-apis/hire/v1/jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 1061004, "msg": "hire module not enabled"
            })))
            .mount(&server)
            .await;
        let err = api.hire_list_jobs(None, None).await.unwrap_err();
        assert!(err.to_string().contains("1061004"), "{}", err);
    }
}
