//! wiki (知识库 / Lark Wiki) REST methods.
//!
//! Extends [`FeishuApi`] with the single wiki endpoint used by C4 —
//! `wiki_get_node`. The typical agent flow is "user gives me a wiki link,
//! help me read the content": resolving the wiki token to its underlying
//! `obj_token` + `obj_type` lets the agent then call e.g.
//! `feishu_docx_get_blocks` to actually read the body. Wiki node creation
//! / child listing are deferred to v0.3+ per the
//! [`feishu-business-tools.md`](../../../../../docs/plans/feishu-business-tools.md)
//! §9 P3 follow-up table.
//!
//! Reference:
//! - <https://open.feishu.cn/document/server-docs/docs/wiki-v2/space-node/get_node>

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use super::api::FeishuApi;

// ── Response types ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WikiNode {
    /// Wiki space the node belongs to.
    pub space_id: String,
    /// The wiki node's own token (e.g. `wikcnAbC...`). Stable; identifies
    /// the position in the wiki tree.
    pub node_token: String,
    /// The underlying object token (e.g. `doxcnXxx` for a docx-backed wiki
    /// page). Use this with `feishu_docx_*` / `feishu_bitable_*` tools to
    /// read or modify the actual content.
    pub obj_token: String,
    /// Object type behind the wiki node — `docx` / `doc` / `sheet` /
    /// `mindnote` / `bitable` / `file` / `slides` / `wiki`.
    pub obj_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_node_token: Option<String>,
    /// `origin` (real node) vs `shortcut` (link to another node).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_node_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_space_id: Option<String>,
    #[serde(default)]
    pub has_child: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obj_create_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obj_edit_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_create_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WikiNodeData {
    node: WikiNode,
}

// ── Public methods on FeishuApi ─────────────────────────────────

impl FeishuApi {
    /// `GET /open-apis/wiki/v2/spaces/get_node` — resolve a wiki token to
    /// its node metadata (space, parent, underlying obj_token + obj_type).
    ///
    /// `obj_type` is optional and defaults to `"wiki"` per Feishu's API; if
    /// the token is the underlying obj (e.g. `doxcnXxx` instead of
    /// `wikcnXxx`), the caller should pass the matching obj_type.
    pub async fn wiki_get_node(
        &self,
        token: &str,
        obj_type: Option<&str>,
    ) -> Result<WikiNode> {
        let mut url = format!("{}/open-apis/wiki/v2/spaces/get_node", self.base_url());
        let mut params: Vec<(&str, String)> = vec![("token", token.to_string())];
        if let Some(t) = obj_type {
            params.push(("obj_type", t.to_string()));
        }
        super::api::append_query(&mut url, &params);
        let resp = self
            .authorized_request(reqwest::Method::GET, &url)
            .await?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to GET wiki_get_node: {}", e))?;
        let data: WikiNodeData = self
            .parse_envelope(resp, "wiki_get_node")
            .await?
            .ok_or_else(|| anyhow!("wiki_get_node response missing 'data'"))?;
        Ok(data.node)
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use super::super::api::test_support::mock_api;

    #[tokio::test]
    async fn get_node_returns_resolved_obj_token() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;

        Mock::given(method("GET"))
            .and(path("/open-apis/wiki/v2/spaces/get_node"))
            .and(query_param("token", "wikcnAbc"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "success",
                "data": {
                    "node": {
                        "space_id": "7000000000",
                        "node_token": "wikcnAbc",
                        "obj_token": "doxcnXyz",
                        "obj_type": "docx",
                        "title": "OKR 周报",
                        "has_child": true,
                        "node_type": "origin"
                    }
                }
            })))
            .mount(&server)
            .await;

        let node = api.wiki_get_node("wikcnAbc", None).await.unwrap();
        assert_eq!(node.space_id, "7000000000");
        assert_eq!(node.obj_token, "doxcnXyz");
        assert_eq!(node.obj_type, "docx");
        assert_eq!(node.title.as_deref(), Some("OKR 周报"));
        assert!(node.has_child);
        assert_eq!(node.node_type.as_deref(), Some("origin"));
    }

    #[tokio::test]
    async fn get_node_passes_obj_type_when_provided() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;

        Mock::given(method("GET"))
            .and(path("/open-apis/wiki/v2/spaces/get_node"))
            .and(query_param("token", "doxcnY"))
            .and(query_param("obj_type", "docx"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "success",
                "data": {
                    "node": {
                        "space_id": "70",
                        "node_token": "wikcnY",
                        "obj_token": "doxcnY",
                        "obj_type": "docx"
                    }
                }
            })))
            .mount(&server)
            .await;

        let node = api
            .wiki_get_node("doxcnY", Some("docx"))
            .await
            .unwrap();
        assert_eq!(node.obj_token, "doxcnY");
    }

    #[tokio::test]
    async fn get_node_propagates_envelope_error() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;

        Mock::given(method("GET"))
            .and(path("/open-apis/wiki/v2/spaces/get_node"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 99991663,
                "msg": "wiki node not found"
            })))
            .mount(&server)
            .await;

        let err = api.wiki_get_node("wikcnGone", None).await.unwrap_err();
        assert!(err.to_string().contains("99991663"), "{}", err);
    }
}
