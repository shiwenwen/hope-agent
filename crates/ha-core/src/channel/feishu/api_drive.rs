//! drive (云盘 / Lark Drive) REST methods.
//!
//! Extends [`FeishuApi`] with the three core drive endpoints used by the
//! C3 tools (list / upload / download). Upload is capped at 20 MB —
//! Feishu's `medias/upload_all` endpoint enforces this server-side; we
//! short-circuit before sending so over-sized requests don't waste
//! bandwidth. Larger files need the v2 segmented upload protocol
//! (`upload_prepare` / `upload_part` / `upload_finish`), deferred to v0.3+.
//!
//! References:
//! - <https://open.feishu.cn/document/server-docs/docs/drive-v1/folder/list>
//! - <https://open.feishu.cn/document/server-docs/docs/drive-v1/media/upload_all>
//! - <https://open.feishu.cn/document/server-docs/docs/drive-v1/media/download>

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use super::api::{build_part, FeishuApi};

/// Hard cap for `medias/upload_all`. Feishu rejects requests above this on
/// the server side; we mirror the limit here so we error before uploading.
pub const DRIVE_UPLOAD_MAX_BYTES: u64 = 20 * 1024 * 1024;

// ── Response types ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DriveFile {
    pub token: String,
    pub name: String,
    /// `doc` / `sheet` / `mindnote` / `bitable` / `file` / `folder` / `shortcut` — see Feishu docs.
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DriveFilesPage {
    #[serde(default)]
    pub files: Vec<DriveFile>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DriveUploadResult {
    pub file_token: String,
}

// ── Public methods on FeishuApi ─────────────────────────────────

impl FeishuApi {
    /// `GET /open-apis/drive/v1/files` — list folder contents.
    /// `folder_token = None` lists the user's drive root.
    pub async fn drive_list_files(
        &self,
        folder_token: Option<&str>,
        page_token: Option<&str>,
        page_size: Option<u32>,
    ) -> Result<DriveFilesPage> {
        let mut url = format!("{}/open-apis/drive/v1/files", self.base_url());
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(t) = folder_token {
            params.push(("folder_token", t.to_string()));
        }
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
            .map_err(|e| anyhow!("Failed to GET drive_list_files: {}", e))?;
        Ok(self
            .parse_envelope::<DriveFilesPage>(resp, "drive_list_files")
            .await?
            .unwrap_or_default())
    }

    /// `POST /open-apis/drive/v1/medias/upload_all` — multipart upload of a
    /// single binary blob (≤ 20 MB). Returns the new `file_token`.
    ///
    /// `parent_type` is typically `"explorer"` for normal drive files. For
    /// uploads bound to a docx (image inside a document), use
    /// `"docx_image"` and pass the docx's image-block ID as `parent_node`.
    /// See Feishu docs for the full enum.
    pub async fn drive_upload_media(
        &self,
        file_name: &str,
        parent_type: &str,
        parent_node: &str,
        bytes: Vec<u8>,
        mime: Option<&str>,
    ) -> Result<DriveUploadResult> {
        let size = bytes.len() as u64;
        if size == 0 {
            return Err(anyhow!("drive_upload_media: file is empty"));
        }
        if size > DRIVE_UPLOAD_MAX_BYTES {
            return Err(anyhow!(
                "drive_upload_media: file size {} bytes exceeds 20 MB limit. \
                 Files >20 MB need segmented upload v2 (upload_prepare / upload_part / upload_finish), \
                 deferred to a future release.",
                size
            ));
        }
        let url = format!("{}/open-apis/drive/v1/medias/upload_all", self.base_url());
        let mime = mime.unwrap_or("application/octet-stream");
        let form = reqwest::multipart::Form::new()
            .text("file_name", file_name.to_string())
            .text("parent_type", parent_type.to_string())
            .text("parent_node", parent_node.to_string())
            .text("size", size.to_string())
            .part("file", build_part(bytes, file_name, mime, "drive media")?);
        let data: DriveUploadResult = self.upload_multipart(&url, form, "drive media").await?;
        Ok(data)
    }

    /// `GET /open-apis/drive/v1/medias/{file_token}/download` — fetch the
    /// raw binary content of a previously uploaded media file.
    pub async fn drive_download_media(&self, file_token: &str) -> Result<Vec<u8>> {
        let url = format!(
            "{}/open-apis/drive/v1/medias/{}/download",
            self.base_url(),
            file_token
        );
        let resp = self
            .authorized_request(reqwest::Method::GET, &url)
            .await?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to GET drive_download_media: {}", e))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp
                .text()
                .await
                .map_err(|e| anyhow!("Failed to read drive download error body: {}", e))?;
            return Err(anyhow!(
                "Feishu drive_download_media HTTP {} (file_token='{}'): {}",
                status,
                file_token,
                crate::truncate_utf8(&body, 512)
            ));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| anyhow!("Failed to read drive media bytes: {}", e))?;
        Ok(bytes.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use super::super::api::test_support::mock_api;

    #[tokio::test]
    async fn list_files_passes_pagination() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;

        Mock::given(method("GET"))
            .and(path("/open-apis/drive/v1/files"))
            .and(query_param("folder_token", "fldcnRoot"))
            .and(query_param("page_size", "20"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "success",
                "data": {
                    "files": [
                        {"token": "fileA", "name": "report.pdf", "type": "file"},
                        {"token": "fldcnSub", "name": "Subfolder", "type": "folder"}
                    ],
                    "has_more": false
                }
            })))
            .mount(&server)
            .await;

        let page = api
            .drive_list_files(Some("fldcnRoot"), None, Some(20))
            .await
            .unwrap();
        assert_eq!(page.files.len(), 2);
        assert_eq!(page.files[0].kind.as_deref(), Some("file"));
        assert!(!page.has_more);
    }

    #[tokio::test]
    async fn upload_media_returns_file_token() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;

        Mock::given(method("POST"))
            .and(path("/open-apis/drive/v1/medias/upload_all"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "success",
                "data": {"file_token": "boxcnNew123"}
            })))
            .mount(&server)
            .await;

        let result = api
            .drive_upload_media(
                "hello.txt",
                "explorer",
                "fldcnRoot",
                b"hello world".to_vec(),
                Some("text/plain"),
            )
            .await
            .unwrap();
        assert_eq!(result.file_token, "boxcnNew123");
    }

    #[tokio::test]
    async fn upload_media_rejects_empty() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        let err = api
            .drive_upload_media("x.bin", "explorer", "fldcn", Vec::new(), None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("empty"), "{}", err);
    }

    #[tokio::test]
    async fn upload_media_rejects_over_limit() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        let too_big = vec![0u8; (DRIVE_UPLOAD_MAX_BYTES + 1) as usize];
        let err = api
            .drive_upload_media("x.bin", "explorer", "fldcn", too_big, None)
            .await
            .unwrap_err();
        let s = err.to_string();
        assert!(s.contains("20 MB"), "{}", s);
    }

    #[tokio::test]
    async fn download_media_returns_raw_bytes() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;

        Mock::given(method("GET"))
            .and(path("/open-apis/drive/v1/medias/boxcnA/download"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"binary content"))
            .mount(&server)
            .await;

        let bytes = api.drive_download_media("boxcnA").await.unwrap();
        assert_eq!(bytes, b"binary content");
    }

    #[tokio::test]
    async fn download_media_propagates_http_error() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;

        Mock::given(method("GET"))
            .and(path("/open-apis/drive/v1/medias/boxcnGone/download"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .mount(&server)
            .await;

        let err = api.drive_download_media("boxcnGone").await.unwrap_err();
        assert!(err.to_string().contains("HTTP 404"), "{}", err);
    }

    #[tokio::test]
    async fn list_files_propagates_envelope_error() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;

        Mock::given(method("GET"))
            .and(path("/open-apis/drive/v1/files"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 99991672,
                "msg": "drive scope missing"
            })))
            .mount(&server)
            .await;

        let err = api
            .drive_list_files(None, None, None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("99991672"), "{}", err);
    }
}
