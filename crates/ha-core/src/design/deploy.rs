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
    // 部署前预检：空 / 超限直接拒（不上传半成品）。
    let pf = preflight_report(&html);
    if !pf.ok {
        bail!("部署预检未通过：{}", pf.errors.join("；"));
    }
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
    // 记部署历史（失败不阻断部署结果）。
    if let Err(e) = db.record_deployment(
        artifact_id,
        "cloudflare",
        &url,
        &chrono::Utc::now().to_rfc3339(),
    ) {
        crate::app_warn!("design", "deploy", "record deployment history failed: {e}");
    }
    Ok(url)
}

/// 部署 URL 就绪探测结果（W3-J/W3-L）：`ready` 前端显示「链接生效中」+ 轮询重试。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployReadiness {
    /// 边缘已就绪（末跳 2xx/3xx）。
    pub ready: bool,
    /// 探测到的 HTTP 状态码（网络层错误 = None，视为未就绪）。
    pub status: Option<u16>,
}

/// 探测部署 URL 是否已生效（CF Pages / Vercel 边缘传播有延迟，部署成功后立即打开可能 404）。
///
/// **SSRF**：目标是用户的**公网**站点（host 随部署商 / 自定义域可变），用 `Default` 策略
/// （放行公网、拦私网 / 环回 / 元数据），**不复用部署出站的 Strict + API allowlist**。
/// 网络层错误（DNS 未生效 / 连接被拒）不算硬失败——回 not-ready 让前端继续轮询。
pub async fn probe_deploy_ready(url: &str) -> Result<DeployReadiness> {
    crate::security::ssrf::check_url(url, crate::security::ssrf::SsrfPolicy::Default, &[])
        .await
        .with_context(|| format!("SSRF check failed for {url}"))?;
    // **不跟随跳转（红线）**：`check_url` 只校验首个 URL；若跟随默认 10 跳，一个公网 URL 可
    // `302 → 169.254.169.254 / 内网` 把探测变成盲 SSRF 内网扫描（跳转 hop 不再过 SSRF）。就绪
    // 探测本就只需知道「边缘有响应」——3xx 本身即响应信号（下方按 redirection 也算 ready），故
    // 直接 `Policy::none()` 彻底断掉跳转向量，无需 per-hop 复核。
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| anyhow!("build probe client: {e}"))?;
    match http.get(url).send().await {
        Ok(resp) => {
            let s = resp.status();
            Ok(DeployReadiness {
                ready: s.is_success() || s.is_redirection(),
                status: Some(s.as_u16()),
            })
        }
        Err(_) => Ok(DeployReadiness {
            ready: false,
            status: None,
        }),
    }
}

/// 部署单文件上限（保守，覆盖 CF Pages / Vercel 内联单文件安全区）。
pub(crate) const MAX_DEPLOY_BYTES: usize = 25 * 1024 * 1024;

/// 部署预检报告：`errors` 阻断（空 / 超限），`warnings` 非阻断（破坏自包含的外部资源引用等）。
/// CF / Vercel 共用；owner 平面读，部署时 errors 非空则 fail-fast。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightReport {
    /// 无阻断问题（可部署）。
    pub ok: bool,
    pub size_bytes: usize,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

/// 统计破坏自包含的**资源**引用（`src=`/`url(`/`@import` 指向 http(s)）。锚点 `href="http…"`
/// 是合法导航、**不计**（只查资源加载）。产物本应全部 data: URI 内联。
pub(crate) fn count_external_refs(html: &str) -> usize {
    let lower = html.to_ascii_lowercase();
    let needles = [
        "src=\"http",
        "src='http",
        "url(http",
        "url('http",
        "url(\"http",
        "@import \"http",
        "@import 'http",
    ];
    needles.iter().map(|n| lower.matches(n).count()).sum()
}

/// 纯函数：从干净 HTML 算预检（无 IO，便于单测）。
pub(crate) fn preflight_report(html: &str) -> PreflightReport {
    let size_bytes = html.len();
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    let trimmed = html.trim();
    if trimmed.is_empty() {
        errors.push("产物为空，无法部署".to_string());
    } else if !trimmed.to_ascii_lowercase().contains("<html") {
        warnings.push("缺少 <html> 根标签".to_string());
    }
    if size_bytes > MAX_DEPLOY_BYTES {
        errors.push(format!(
            "产物过大（{} MB，上限 {} MB）",
            size_bytes / 1024 / 1024,
            MAX_DEPLOY_BYTES / 1024 / 1024
        ));
    }
    let ext = count_external_refs(html);
    if ext > 0 {
        warnings.push(format!("检测到 {ext} 处外部资源引用（应内联为 data: URI）"));
    }
    PreflightReport {
        ok: errors.is_empty(),
        size_bytes,
        warnings,
        errors,
    }
}

/// owner 平面：对产物做部署预检（渲染干净 HTML → 报告）。
pub fn preflight_artifact(artifact_id: &str) -> Result<PreflightReport> {
    let a = super::service::open_db()?
        .get_artifact(artifact_id)?
        .context("artifact not found")?;
    let html = super::service::render_clean_html_for_artifact(&a)?;
    Ok(preflight_report(&html))
}

/// 自定义域名基本格式校验（防注入 CF API 垃圾 / 路径穿越）。宽松但拒明显非法。
pub(crate) fn valid_domain(d: &str) -> bool {
    let d = d.trim();
    !d.is_empty()
        && d.len() <= 253
        && d.contains('.')
        && !d.starts_with('.')
        && !d.ends_with('.')
        && d.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// CF Pages 自定义域名 + 验证状态（`pending` / `active` / `error` …）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomDomain {
    pub name: String,
    pub status: String,
}

fn parse_domain(v: &serde_json::Value, fallback: &str) -> CustomDomain {
    CustomDomain {
        name: v
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or(fallback)
            .to_string(),
        status: v
            .get("status")
            .and_then(|x| x.as_str())
            .unwrap_or("pending")
            .to_string(),
    }
}

/// 给产物的 CF Pages 项目绑定自定义域名（owner 平面）。返回域名 + 初始验证状态
/// （用户须自行把该域名 CNAME 到 `<name>.pages.dev` 才会转 `active`）。
pub async fn bind_custom_domain(artifact_id: &str, domain: &str) -> Result<CustomDomain> {
    let domain = domain.trim();
    if !valid_domain(domain) {
        bail!("非法域名：{domain}");
    }
    let cfg = load_cf_config()?
        .filter(|c| !c.api_token.is_empty() && !c.account_id.is_empty())
        .context("Cloudflare 未配置：需 API token + Account ID")?;
    let a = super::service::open_db()?
        .get_artifact(artifact_id)?
        .context("artifact not found")?;
    let name = project_name_for(&a.title, &a.id);
    let url = format!(
        "{CF_API}/accounts/{}/pages/projects/{name}/domains",
        cfg.account_id
    );
    guard(&url).await?;
    let resp = client()?
        .post(&url)
        .bearer_auth(&cfg.api_token)
        .json(&serde_json::json!({ "name": domain }))
        .send()
        .await
        .context("bind custom domain")?;
    let result = cf_json(resp, "bind custom domain").await?;
    crate::app_info!("design", "deploy", "bound custom domain {domain} to {name}");
    Ok(parse_domain(&result, domain))
}

/// 列出产物 CF Pages 项目已绑定的自定义域名及验证状态（owner 平面）。
pub async fn list_custom_domains(artifact_id: &str) -> Result<Vec<CustomDomain>> {
    let cfg = match load_cf_config()? {
        Some(c) if !c.api_token.is_empty() && !c.account_id.is_empty() => c,
        _ => return Ok(Vec::new()),
    };
    let a = super::service::open_db()?
        .get_artifact(artifact_id)?
        .context("artifact not found")?;
    let name = project_name_for(&a.title, &a.id);
    let url = format!(
        "{CF_API}/accounts/{}/pages/projects/{name}/domains",
        cfg.account_id
    );
    guard(&url).await?;
    let resp = client()?
        .get(&url)
        .bearer_auth(&cfg.api_token)
        .send()
        .await
        .context("list custom domains")?;
    // 项目尚未创建（从未部署）→ 404，视为无域名而非错误。
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Vec::new());
    }
    let result = cf_json(resp, "list custom domains").await?;
    Ok(result
        .as_array()
        .map(|arr| arr.iter().map(|v| parse_domain(v, "")).collect())
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preflight_flags_empty_and_external_refs() {
        // 空 → 阻断。
        let empty = preflight_report("   ");
        assert!(!empty.ok);
        assert!(!empty.errors.is_empty());
        // 正常自包含 → ok 且无外部引用告警。
        let good =
            preflight_report("<html><body><img src=\"data:image/png;base64,AAA\"></body></html>");
        assert!(good.ok, "{:?}", good.errors);
        assert_eq!(
            count_external_refs("<img src=\"data:image/png;base64,AAA\">"),
            0
        );
        // 外部资源引用 → 非阻断告警（仍可部署）。
        let ext = preflight_report("<html><img src=\"https://cdn.example.com/a.png\"></html>");
        assert!(ext.ok, "外部引用只告警不阻断");
        assert!(!ext.warnings.is_empty());
        // 锚点 href 不计为资源引用。
        assert_eq!(
            count_external_refs("<a href=\"https://example.com\">x</a>"),
            0
        );
        // CSS url() 计入。
        assert_eq!(
            count_external_refs("body{background:url(https://x/y.png)}"),
            1
        );
    }

    #[test]
    fn valid_domain_accepts_real_rejects_garbage() {
        assert!(valid_domain("example.com"));
        assert!(valid_domain("deck.my-brand.io"));
        assert!(!valid_domain("nodot"));
        assert!(!valid_domain("bad domain.com"));
        assert!(!valid_domain("a.com/../etc"));
        assert!(!valid_domain(".leading.com"));
        assert!(!valid_domain(""));
    }

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
