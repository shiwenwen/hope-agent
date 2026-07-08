//! Cloudflare Pages 一键部署（B7-2，opt-in）。
//!
//! 产物**自包含**（data-uri 资产内嵌）故整站 = 单个 `index.html` → CF 直传序列大幅简化：
//! ensure project → upload-token(JWT) → blake3 hash → check-missing → upload → upsert-hashes →
//! create deployment(multipart manifest) → 返回 `<name>.pages.dev`。
//!
//! **安全红线**：① 所有出站**只到 `api.cloudflare.com`**（`ssrf::check_url` Strict + allowlist，
//! 每个 URL 先校验后请求）；② API token **0600** 存 `credentials/cloudflare.json`，GUI 读脱敏
//! （回 mask 哨兵，从不回传明文）；③ owner 平面显式触发，**后台自主维护绝不部署**；④ 只上传
//! 本产物的干净 HTML，不抓取/上传任何外部引用（产物本就自包含）。

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;

const CF_API: &str = "https://api.cloudflare.com/client/v4";
const CF_HOST: &str = "api.cloudflare.com";
/// GUI 回填该哨兵 = 保留已存 token（不改）。
pub const TOKEN_MASK: &str = "__cf_saved__";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudflareConfig {
    pub api_token: String,
    pub account_id: String,
}

fn cf_config_path() -> Result<std::path::PathBuf> {
    Ok(crate::paths::credentials_dir()?.join("cloudflare.json"))
}

pub fn load_cf_config() -> Result<Option<CloudflareConfig>> {
    let path = cf_config_path()?;
    match std::fs::read(&path) {
        Ok(b) => Ok(Some(
            serde_json::from_slice(&b).context("parse cloudflare.json")?,
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow!("read cloudflare.json: {e}")),
    }
}

/// owner 保存：token 为 mask → 保留原 token（GUI 只改 account）。清空 account/token 允许。
pub fn save_cf_config(api_token: &str, account_id: &str) -> Result<()> {
    let token = if api_token == TOKEN_MASK {
        load_cf_config()?.map(|c| c.api_token).unwrap_or_default()
    } else {
        api_token.trim().to_string()
    };
    let cfg = CloudflareConfig {
        api_token: token,
        account_id: account_id.trim().to_string(),
    };
    let bytes = serde_json::to_vec_pretty(&cfg)?;
    crate::platform::write_secure_file(&cf_config_path()?, &bytes)
        .map_err(|e| anyhow!("write cloudflare.json: {e}"))?;
    crate::app_info!("design", "deploy", "saved cloudflare deploy config");
    Ok(())
}

/// GUI 读：**token 脱敏**（有 token 只回 `has_token` + mask 哨兵，绝不回明文）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfConfigPublic {
    pub account_id: String,
    pub has_token: bool,
    pub token_mask: String,
}

pub fn public_cf_config() -> Result<CfConfigPublic> {
    let cfg = load_cf_config()?;
    Ok(CfConfigPublic {
        account_id: cfg
            .as_ref()
            .map(|c| c.account_id.clone())
            .unwrap_or_default(),
        has_token: cfg.as_ref().is_some_and(|c| !c.api_token.is_empty()),
        token_mask: TOKEN_MASK.to_string(),
    })
}

/// DNS-safe 项目名 `ha-<slug>-<id8>`（小写字母数字 + `-`，≤63，去首尾 `-`）。CF 项目名不可变
/// 故按产物稳定派生（同产物重部署命中同项目、覆盖同 pages.dev 子域）。
pub(crate) fn project_name_for(title: &str, artifact_id: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for c in title.to_ascii_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    let slug = slug.trim_matches('-');
    let id8: String = artifact_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect();
    let mut name = format!("ha-{slug}-{id8}");
    name.truncate(63);
    let name = name.trim_matches('-').to_string();
    if name.is_empty() {
        format!("ha-{id8}")
    } else {
        name
    }
}

/// blake3(base64(data)+ext) 前 32 hex——CF Pages 资产键（对齐参照 `cloudflarePagesAssetHash`）。
fn asset_hash(b64: &str, ext: &str) -> String {
    let mut h = blake3::Hasher::new();
    h.update(b64.as_bytes());
    h.update(ext.as_bytes());
    h.finalize().to_hex()[..32].to_string()
}

/// 出站前 SSRF：**只放行 `api.cloudflare.com`**（Strict = 公网 only，再叠 host allowlist）。
async fn guard(url: &str) -> Result<()> {
    crate::security::ssrf::check_url(
        url,
        crate::security::ssrf::SsrfPolicy::Strict,
        &[CF_HOST.to_string()],
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

/// CF v4 响应 `{ success, errors, result }`——非 success 抛错带首条 error 消息。
async fn cf_json(resp: reqwest::Response, ctx: &str) -> Result<serde_json::Value> {
    let status = resp.status();
    let body: serde_json::Value = resp
        .json()
        .await
        .with_context(|| format!("{ctx}: parse response"))?;
    if body.get("success").and_then(|v| v.as_bool()) != Some(true) {
        let msg = body
            .get("errors")
            .and_then(|e| e.as_array())
            .and_then(|a| a.first())
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        bail!("{ctx} failed (HTTP {status}): {msg}");
    }
    Ok(body
        .get("result")
        .cloned()
        .unwrap_or(serde_json::Value::Null))
}

/// 部署产物到 CF Pages，返回 `https://<name>.pages.dev`。owner 平面显式调用。
pub async fn deploy_artifact(artifact_id: &str) -> Result<String> {
    let cfg = load_cf_config()?
        .filter(|c| !c.api_token.is_empty() && !c.account_id.is_empty())
        .context("Cloudflare 未配置：需 API token + Account ID")?;
    let token = &cfg.api_token;
    let acct = &cfg.account_id;

    // 干净自包含 HTML（无 bridge/oid）。
    let db = super::service::open_db()?;
    let a = db
        .get_artifact(artifact_id)?
        .context("artifact not found")?;
    let html = super::service::render_clean_html_for_artifact(&a)?;
    let name = project_name_for(&a.title, &a.id);
    let b64 = base64::engine::general_purpose::STANDARD.encode(html.as_bytes());
    let hash = asset_hash(&b64, ".html");

    let http = client()?;
    crate::app_info!(
        "design",
        "deploy",
        "deploying artifact {artifact_id} to CF project {name}"
    );

    // ① ensure project（GET；404 → POST 建；已存在容忍）。
    let proj_url = format!("{CF_API}/accounts/{acct}/pages/projects/{name}");
    guard(&proj_url).await?;
    let get = http.get(&proj_url).bearer_auth(token).send().await?;
    if get.status() == reqwest::StatusCode::NOT_FOUND {
        let create_url = format!("{CF_API}/accounts/{acct}/pages/projects");
        guard(&create_url).await?;
        let resp = http
            .post(&create_url)
            .bearer_auth(token)
            .json(&json!({ "name": name, "production_branch": "main" }))
            .send()
            .await?;
        // 并发 / 已存在 → 容忍（后续步骤仍可用）。
        if !resp.status().is_success() {
            let _ = cf_json(resp, "create project").await; // 记录但不硬失败于「已存在」
        }
    } else if !get.status().is_success() {
        let _ = cf_json(get, "get project").await?;
    }

    // ② upload-token（JWT，仅用于资产端点）。
    let ut_url = format!("{CF_API}/accounts/{acct}/pages/projects/{name}/upload-token");
    guard(&ut_url).await?;
    let ut = cf_json(
        http.get(&ut_url).bearer_auth(token).send().await?,
        "get upload token",
    )
    .await?;
    let jwt = ut
        .get("jwt")
        .and_then(|v| v.as_str())
        .context("upload-token: no jwt in result")?
        .to_string();

    // ③ check-missing（缺失才传，省流量）。
    let check_url = format!("{CF_API}/pages/assets/check-missing");
    guard(&check_url).await?;
    let missing = cf_json(
        http.post(&check_url)
            .bearer_auth(&jwt)
            .json(&json!({ "hashes": [hash] }))
            .send()
            .await?,
        "check missing",
    )
    .await?;
    let need_upload = missing
        .as_array()
        .map(|a| a.iter().any(|h| h.as_str() == Some(hash.as_str())))
        .unwrap_or(true);

    // ④ upload（若缺）。
    if need_upload {
        let up_url = format!("{CF_API}/pages/assets/upload");
        guard(&up_url).await?;
        cf_json(
            http.post(&up_url)
                .bearer_auth(&jwt)
                .json(&json!([{
                    "key": hash,
                    "value": b64,
                    "metadata": { "contentType": "text/html" },
                    "base64": true
                }]))
                .send()
                .await?,
            "upload asset",
        )
        .await?;
    }

    // ⑤ upsert-hashes。
    let upsert_url = format!("{CF_API}/pages/assets/upsert-hashes");
    guard(&upsert_url).await?;
    cf_json(
        http.post(&upsert_url)
            .bearer_auth(&jwt)
            .json(&json!({ "hashes": [hash] }))
            .send()
            .await?,
        "upsert hashes",
    )
    .await?;

    // ⑥ create deployment（multipart：manifest + branch）。
    let deploy_url = format!("{CF_API}/accounts/{acct}/pages/projects/{name}/deployments");
    guard(&deploy_url).await?;
    let manifest = json!({ "/index.html": hash }).to_string();
    let form = reqwest::multipart::Form::new()
        .text("manifest", manifest)
        .text("branch", "main");
    let dep = cf_json(
        http.post(&deploy_url)
            .bearer_auth(token)
            .multipart(form)
            .send()
            .await?,
        "create deployment",
    )
    .await?;
    // pages.dev 子域：优先 result.url，回退派生。
    let url = dep
        .get("url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("https://{name}.pages.dev"));
    crate::app_info!(
        "design",
        "deploy",
        "deployed artifact {artifact_id} -> {url}"
    );
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_name_is_dns_safe_and_bounded() {
        let n = project_name_for("我的 Pricing Page!!", "abcd1234-ef56-7890");
        assert!(n.starts_with("ha-"), "{n}");
        assert!(n.len() <= 63);
        assert!(
            n.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "非 DNS-safe: {n}"
        );
        assert!(!n.starts_with('-') && !n.ends_with('-'));
        // 全非 ASCII 标题也有非空名。
        let n2 = project_name_for("演示", "zz99");
        assert!(!n2.is_empty() && n2.contains("zz99"));
    }

    #[test]
    fn token_mask_preserves_secret() {
        // save 逻辑：mask → 保留（此处只验哨兵常量稳定，避免污染真实 credentials 目录）。
        assert_eq!(TOKEN_MASK, "__cf_saved__");
    }

    #[test]
    fn asset_hash_is_32_hex_stable() {
        let h = asset_hash("aGVsbG8=", ".html");
        assert_eq!(h.len(), 32);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(h, asset_hash("aGVsbG8=", ".html"), "hash 应确定");
        assert_ne!(h, asset_hash("aGVsbG8=", ".css"), "扩展名进 hash");
    }

    #[tokio::test]
    async fn ssrf_guard_blocks_internal_targets() {
        // 防御纵深红线：即便请求被构造指向内网 / 云元数据端点也必拒（字面 IP → classify_ip
        // 确定性拒，离线可测）。实际部署 URL host 恒为硬编码 api.cloudflare.com（acct/name 只进
        // path 不改 authority），故主约束在硬编码 host，guard 兜底 SSRF。
        assert!(guard("http://169.254.169.254/latest/meta-data")
            .await
            .is_err());
        assert!(guard("http://127.0.0.1:8080/x").await.is_err());
        assert!(guard("http://10.0.0.1/x").await.is_err());
    }
}
