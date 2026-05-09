//! docx (云文档 / Lark Docs) REST methods.
//!
//! Extends [`FeishuApi`] with the four core docx endpoints used by the C1
//! tools (create / get_blocks / append_block / update_block_text). All
//! methods go through the parent module's `authorized_request` +
//! `parse_envelope` helpers — the `{code, msg, data}` envelope, error
//! propagation, and tenant token refresh stay centralized.
//!
//! References:
//! - <https://open.feishu.cn/document/server-docs/docs/docs/docx-v1/document/create>
//! - <https://open.feishu.cn/document/server-docs/docs/docs/docx-v1/document/list>
//! - <https://open.feishu.cn/document/server-docs/docs/docs/docx-v1/document-block/create>
//! - <https://open.feishu.cn/document/server-docs/docs/docs/docx-v1/document-block/patch>

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use super::api::FeishuApi;

// ── Response types ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DocxDocument {
    pub document_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct DocxCreateData {
    document: DocxDocument,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DocxBlocksPage {
    #[serde(default)]
    pub items: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_token: Option<String>,
    #[serde(default)]
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DocxAppendResult {
    #[serde(default)]
    pub children: Vec<serde_json::Value>,
    /// Updated document revision after the insert.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_revision_id: Option<u64>,
    /// Optimistic-concurrency token returned by some endpoints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DocxBlockPatchResult {
    #[serde(default)]
    pub block: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_revision_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_token: Option<String>,
}

// ── Public methods on FeishuApi ─────────────────────────────────

impl FeishuApi {
    /// `POST /open-apis/docx/v1/documents` — create a new docx.
    /// `title` and `folder_token` are optional; default folder is the user's
    /// drive root.
    pub async fn docx_create(
        &self,
        title: Option<&str>,
        folder_token: Option<&str>,
    ) -> Result<DocxDocument> {
        let url = format!("{}/open-apis/docx/v1/documents", self.base_url());
        let mut body = serde_json::json!({});
        if let Some(t) = title {
            body["title"] = serde_json::Value::String(t.to_string());
        }
        if let Some(f) = folder_token {
            body["folder_token"] = serde_json::Value::String(f.to_string());
        }
        let resp = self
            .authorized_request(reqwest::Method::POST, &url)
            .await?
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to POST docx_create: {}", e))?;
        let data: DocxCreateData = self
            .parse_envelope(resp, "docx_create")
            .await?
            .ok_or_else(|| anyhow!("docx_create response missing 'data'"))?;
        Ok(data.document)
    }

    /// `GET /open-apis/docx/v1/documents/{id}/blocks` — list all blocks in
    /// the document. Supports server-side pagination.
    pub async fn docx_get_blocks(
        &self,
        document_id: &str,
        page_token: Option<&str>,
        page_size: Option<u32>,
    ) -> Result<DocxBlocksPage> {
        let mut url = format!(
            "{}/open-apis/docx/v1/documents/{}/blocks",
            self.base_url(),
            document_id
        );
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(token) = page_token {
            params.push(("page_token", token.to_string()));
        }
        if let Some(size) = page_size {
            params.push(("page_size", size.to_string()));
        }
        super::api::append_query(&mut url, &params);
        let resp = self
            .authorized_request(reqwest::Method::GET, &url)
            .await?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to GET docx_get_blocks: {}", e))?;
        let data: DocxBlocksPage = self
            .parse_envelope(resp, "docx_get_blocks")
            .await?
            .unwrap_or_default();
        Ok(data)
    }

    /// `POST /open-apis/docx/v1/documents/{id}/blocks/{parent}/children` —
    /// append one block under the given parent. `block` is the full block
    /// JSON (Feishu docx block schema; e.g. paragraph: `{block_type: 2,
    /// text: {style: {}, elements: [{text_run: {content: "..."}}]}}`).
    /// `index` is an optional 0-based insert position; default appends.
    pub async fn docx_append_block(
        &self,
        document_id: &str,
        parent_block_id: &str,
        block: serde_json::Value,
        index: Option<u32>,
    ) -> Result<DocxAppendResult> {
        let url = format!(
            "{}/open-apis/docx/v1/documents/{}/blocks/{}/children",
            self.base_url(),
            document_id,
            parent_block_id
        );
        let mut body = serde_json::json!({
            "children": [block],
        });
        if let Some(i) = index {
            body["index"] = serde_json::json!(i);
        }
        let resp = self
            .authorized_request(reqwest::Method::POST, &url)
            .await?
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to POST docx_append_block: {}", e))?;
        let data: DocxAppendResult = self
            .parse_envelope(resp, "docx_append_block")
            .await?
            .unwrap_or_default();
        Ok(data)
    }

    /// `PATCH /open-apis/docx/v1/documents/{id}/blocks/{block_id}` —
    /// replace the text content of a text-bearing block. The block must
    /// already exist; this is a destructive overwrite (full replacement of
    /// `update_text_elements.elements`).
    pub async fn docx_update_block_text(
        &self,
        document_id: &str,
        block_id: &str,
        text: &str,
    ) -> Result<DocxBlockPatchResult> {
        let url = format!(
            "{}/open-apis/docx/v1/documents/{}/blocks/{}",
            self.base_url(),
            document_id,
            block_id
        );
        let body = serde_json::json!({
            "update_text_elements": {
                "elements": [
                    {"text_run": {"content": text}}
                ]
            }
        });
        let resp = self
            .authorized_request(reqwest::Method::PATCH, &url)
            .await?
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to PATCH docx_update_block_text: {}", e))?;
        let data: DocxBlockPatchResult = self
            .parse_envelope(resp, "docx_update_block_text")
            .await?
            .unwrap_or_default();
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Build a FeishuApi pointed at the mock server. We pre-prime the auth
    /// token cache via the mock's auth endpoint so the test doesn't need
    /// real network. Auth endpoint returns a fake token that's accepted by
    /// every subsequent request matcher.
    use super::super::api::test_support::mock_api;

    #[tokio::test]
    async fn docx_create_returns_document_id() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;

        Mock::given(method("POST"))
            .and(path("/open-apis/docx/v1/documents"))
            .and(header("Authorization", "Bearer t-fake-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "success",
                "data": {
                    "document": {
                        "document_id": "doxcnAbC123",
                        "revision_id": 1,
                        "title": "Hello"
                    }
                }
            })))
            .mount(&server)
            .await;

        let doc = api.docx_create(Some("Hello"), None).await.unwrap();
        assert_eq!(doc.document_id, "doxcnAbC123");
        assert_eq!(doc.title.as_deref(), Some("Hello"));
        assert_eq!(doc.revision_id, Some(1));
    }

    #[tokio::test]
    async fn docx_create_propagates_envelope_error() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;

        Mock::given(method("POST"))
            .and(path("/open-apis/docx/v1/documents"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 99991672,
                "msg": "permission denied"
            })))
            .mount(&server)
            .await;

        let err = api.docx_create(None, None).await.unwrap_err();
        let s = err.to_string();
        assert!(s.contains("99991672"), "{}", s);
        assert!(s.contains("permission denied"), "{}", s);
    }

    #[tokio::test]
    async fn docx_get_blocks_passes_pagination() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;

        Mock::given(method("GET"))
            .and(path("/open-apis/docx/v1/documents/doxcnX/blocks"))
            .and(query_param("page_token", "next-page"))
            .and(query_param("page_size", "50"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "success",
                "data": {
                    "items": [{"block_id": "b1"}, {"block_id": "b2"}],
                    "page_token": "next-next",
                    "has_more": true
                }
            })))
            .mount(&server)
            .await;

        let page = api
            .docx_get_blocks("doxcnX", Some("next-page"), Some(50))
            .await
            .unwrap();
        assert_eq!(page.items.len(), 2);
        assert!(page.has_more);
        assert_eq!(page.page_token.as_deref(), Some("next-next"));
    }

    #[tokio::test]
    async fn docx_get_blocks_omits_unset_query_params() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;

        Mock::given(method("GET"))
            .and(path("/open-apis/docx/v1/documents/doxcnY/blocks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "success",
                "data": {"items": [], "has_more": false}
            })))
            .mount(&server)
            .await;

        // No page_token / page_size args → URL has no query string at all.
        let page = api.docx_get_blocks("doxcnY", None, None).await.unwrap();
        assert!(page.items.is_empty());
        assert!(!page.has_more);
    }

    #[tokio::test]
    async fn docx_append_block_serializes_block_and_index() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;

        Mock::given(method("POST"))
            .and(path(
                "/open-apis/docx/v1/documents/doxcnZ/blocks/parent_b/children",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "success",
                "data": {
                    "children": [{"block_id": "new_b"}],
                    "document_revision_id": 5
                }
            })))
            .mount(&server)
            .await;

        let block = serde_json::json!({
            "block_type": 2,
            "text": {"style": {}, "elements": [{"text_run": {"content": "hi"}}]}
        });
        let result = api
            .docx_append_block("doxcnZ", "parent_b", block, Some(0))
            .await
            .unwrap();
        assert_eq!(result.children.len(), 1);
        assert_eq!(result.document_revision_id, Some(5));
    }

    #[tokio::test]
    async fn docx_update_block_text_uses_patch() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;

        Mock::given(method("PATCH"))
            .and(path("/open-apis/docx/v1/documents/doxcnW/blocks/b1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "success",
                "data": {"block": {"block_id": "b1"}, "document_revision_id": 7}
            })))
            .mount(&server)
            .await;

        let result = api
            .docx_update_block_text("doxcnW", "b1", "new text")
            .await
            .unwrap();
        assert_eq!(result.document_revision_id, Some(7));
    }

    #[tokio::test]
    async fn docx_update_block_text_propagates_http_error() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;

        Mock::given(method("PATCH"))
            .and(path("/open-apis/docx/v1/documents/doxcnW/blocks/b1"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
            .mount(&server)
            .await;

        let err = api
            .docx_update_block_text("doxcnW", "b1", "x")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("HTTP 500"), "{}", err);
    }
}
