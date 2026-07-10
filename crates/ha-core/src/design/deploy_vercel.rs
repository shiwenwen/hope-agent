//! Vercel 一键部署（多提供商部署第二 provider，opt-in）。与 CF Pages（`deploy.rs`）并列。
//!
//! 产物**自包含**（data-uri 资产内嵌）故整站 = 单个 `index.html` → Vercel `POST /v13/deployments`
//! 内联单文件即可（`files:[{file,data,encoding:utf-8}]` + `target:production`），返回稳定
//! `<name>.vercel.app`。
//!
//! **安全红线**（与 CF 一致）：① 所有出站**只到 `api.vercel.com`**（`ssrf::check_url` Strict +
//! allowlist，每个 URL 先校验后请求）；② API token **0600** 存 `credentials/vercel.json`，GUI 读
//! 脱敏（回 mask 哨兵，从不回传明文）；③ owner 平面显式触发，**后台自主维护绝不部署**；④ 只上传
//! 本产物的干净 HTML，不抓取/上传外部引用（产物本就自包含）。

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const VERCEL_API: &str = "https://api.vercel.com";
const VERCEL_HOST: &str = "api.vercel.com";
/// GUI 回填该哨兵 = 保留已存 token（不改）。
pub const TOKEN_MASK: &str = "__vercel_saved__";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VercelConfig {
    pub api_token: String,
    /// 团队账号可选：非空时所有请求带 `?teamId=`。个人账号留空。
    #[serde(default)]
    pub team_id: String,
}

fn vercel_config_path() -> Result<std::path::PathBuf> {
    Ok(crate::paths::credentials_dir()?.join("vercel.json"))
}

pub fn load_vercel_config() -> Result<Option<VercelConfig>> {
    let path = vercel_config_path()?;
    match std::fs::read(&path) {
        Ok(b) => Ok(Some(
            serde_json::from_slice(&b).context("parse vercel.json")?,
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow!("read vercel.json: {e}")),
    }
}

/// owner 保存：token 为 mask → 保留原 token（GUI 只改 team）。清空 team/token 允许。
pub fn save_vercel_config(api_token: &str, team_id: &str) -> Result<()> {
    let token = if api_token == TOKEN_MASK {
        load_vercel_config()?.map(|c| c.api_token).unwrap_or_default()
    } else {
        api_token.trim().to_string()
    };
    let cfg = VercelConfig {
        api_token: token,
        team_id: team_id.trim().to_string(),
    };
    let bytes = serde_json::to_vec_pretty(&cfg)?;
    crate::platform::write_secure_file(&vercel_config_path()?, &bytes)
        .map_err(|e| anyhow!("write vercel.json: {e}"))?;
    crate::app_info!("design", "deploy", "saved vercel deploy config");
    Ok(())
}

/// GUI 读：**token 脱敏**（有 token 只回 `has_token` + mask 哨兵，绝不回明文）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VercelConfigPublic {
    pub team_id: String,
    pub has_token: bool,
    pub token_mask: String,
}

pub fn public_vercel_config() -> Result<VercelConfigPublic> {
    let cfg = load_vercel_config()?;
    Ok(VercelConfigPublic {
        team_id: cfg.as_ref().map(|c| c.team_id.clone()).unwrap_or_default(),
        has_token: cfg.as_ref().is_some_and(|c| !c.api_token.is_empty()),
        token_mask: TOKEN_MASK.to_string(),
    })
}

/// 出站前 SSRF：**只放行 `api.vercel.com`**（Strict = 公网 only，再叠 host allowlist）。
async fn guard(url: &str) -> Result<()> {
    crate::security::ssrf::check_url(
        url,
        crate::security::ssrf::SsrfPolicy::Strict,
        &[VERCEL_HOST.to_string()],
    )
    .await
    .with_context(|| format!("SSRF check failed for {url}"))?;
    Ok(())
}

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| anyhow!("build http client: {e}"))
}

/// 从 Vercel 部署响应挑一个稳定的产品 URL：优先 `<name>.vercel.app` 形式的 alias，
/// 回退部署 `url`，再回退派生。
fn pick_url(body: &serde_json::Value, name: &str) -> String {
    if let Some(alias) = body.get("alias").and_then(|a| a.as_array()) {
        // production alias 通常是 `<project>.vercel.app`（无 commit hash 后缀）。
        if let Some(stable) = alias
            .iter()
            .filter_map(|v| v.as_str())
            .find(|s| s.ends_with(".vercel.app") && !s.contains('-'))
            .or_else(|| {
                alias
                    .iter()
                    .filter_map(|v| v.as_str())
                    .find(|s| s.ends_with(".vercel.app"))
            })
        {
            return format!("https://{stable}");
        }
    }
    if let Some(u) = body.get("url").and_then(|v| v.as_str()) {
        return format!("https://{u}");
    }
    format!("https://{name}.vercel.app")
}

/// 部署产物到 Vercel，返回 `https://<name>.vercel.app`。owner 平面显式调用。
pub async fn deploy_artifact(artifact_id: &str) -> Result<String> {
    let cfg = load_vercel_config()?
        .filter(|c| !c.api_token.is_empty())
        .context("Vercel 未配置：需 API token")?;

    // 干净自包含 HTML（无 bridge/oid）。
    let db = super::service::open_db()?;
    let a = db
        .get_artifact(artifact_id)?
        .context("artifact not found")?;
    let html = super::service::render_clean_html_for_artifact(&a)?;
    // 复用 CF 的 DNS-safe 项目名派生（同产物重部署命中同项目、覆盖同 vercel.app 子域）。
    let name = super::deploy::project_name_for(&a.title, &a.id);

    let team_q = if cfg.team_id.is_empty() {
        String::new()
    } else {
        format!("?teamId={}", cfg.team_id)
    };
    let url = format!("{VERCEL_API}/v13/deployments{team_q}");
    guard(&url).await?;

    crate::app_info!(
        "design",
        "deploy",
        "deploying artifact {artifact_id} to vercel project {name}"
    );

    let payload = serde_json::json!({
        "name": name,
        "target": "production",
        "files": [{ "file": "index.html", "data": html, "encoding": "utf-8" }],
        "projectSettings": { "framework": serde_json::Value::Null },
    });
    let resp = client()?
        .post(&url)
        .bearer_auth(&cfg.api_token)
        .json(&payload)
        .send()
        .await
        .context("create vercel deployment")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.context("parse vercel response")?;
    if !status.is_success() {
        // Vercel 错误形如 `{ error: { code, message } }`。
        let msg = body
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        bail!("vercel deploy failed (HTTP {status}): {msg}");
    }
    let out = pick_url(&body, &name);
    crate::app_info!("design", "deploy", "deployed artifact {artifact_id} -> {out}");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_mask_is_stable() {
        assert_eq!(TOKEN_MASK, "__vercel_saved__");
    }

    #[test]
    fn pick_url_prefers_stable_alias() {
        let body = serde_json::json!({
            "url": "myproj-a1b2c3.vercel.app",
            "alias": ["myproj.vercel.app", "myproj-a1b2c3.vercel.app"],
        });
        assert_eq!(pick_url(&body, "myproj"), "https://myproj.vercel.app");
    }

    #[test]
    fn pick_url_falls_back_to_deployment_url() {
        let body = serde_json::json!({ "url": "x-a1b2c3.vercel.app" });
        assert_eq!(pick_url(&body, "x"), "https://x-a1b2c3.vercel.app");
    }

    #[test]
    fn pick_url_derives_when_empty() {
        let body = serde_json::json!({});
        assert_eq!(pick_url(&body, "myproj"), "https://myproj.vercel.app");
    }

    #[tokio::test]
    async fn ssrf_guard_blocks_internal_targets() {
        assert!(guard("http://169.254.169.254/latest/meta-data")
            .await
            .is_err());
        assert!(guard("http://127.0.0.1:8080/x").await.is_err());
        assert!(guard("http://10.0.0.1/x").await.is_err());
    }
}
