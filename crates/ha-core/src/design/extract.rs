//! 设计系统反向提取（D2 护城河）。
//!
//! 四通道反向生成品牌设计契约（`SYSTEM.md` + `tokens.json`）：**文本描述** /
//! **本地代码库**（读 CSS / tailwind / theme 样本）/ **URL**（抓原始 HTML）/
//! **截图**（视觉模型，见 `vision.rs`）。"读本地工程提取设计系统" 是云端产品做不到的
//! 本地护城河。见 design-space.md §6.4。

use anyhow::{Context, Result};
use base64::Engine;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// LLM 提取产物。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedSystem {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub system_md: String,
    #[serde(default)]
    pub tokens: BTreeMap<String, String>,
}

/// 核心 token 词表（每个都必须填值，与 `system::expand` / DESIGN.md 互通格式对齐）。
const TOKEN_VOCAB: &str = "--ds-color-bg, --ds-color-fg, --ds-color-primary, --ds-color-secondary, \
--ds-color-accent, --ds-color-muted, --ds-color-border, --ds-color-success, --ds-color-warning, \
--ds-color-danger, --ds-font-sans, --ds-font-serif, --ds-font-mono, --ds-text-base, --ds-text-lg, \
--ds-text-xl, --ds-text-2xl, --ds-text-3xl, --ds-space-2, --ds-space-4, --ds-space-6, --ds-space-8, \
--ds-radius-md, --ds-radius-lg, --ds-shadow-md";

/// 扩展 token（源里明确体现时可补，非必填）——提升表达力而不破坏核心契约。
const TOKEN_VOCAB_EXT: &str = "--ds-text-sm, --ds-text-4xl, --ds-space-1, --ds-space-3, \
--ds-space-12, --ds-radius-sm, --ds-radius-full, --ds-shadow-sm, --ds-shadow-lg, --ds-line-height, \
--ds-line-height-tight, --ds-letter-spacing, --ds-transition, --ds-color-ring, \
--ds-color-primary-contrast, --ds-color-bg-elevated";

/// 材料截断上限（字符）——样式密集的 URL / 代码库需要更大窗口才能抽全 token。
const MATERIAL_CHARS: usize = 40000;

fn build_prompt(source_label: &str, material: &str) -> String {
    format!(
        "You are a brand designer distilling a reusable design system. Based on the {source} below, \
produce a cohesive brand design contract.\n\n\
Return ONLY a JSON object (no prose, no code fence) with keys:\n\
- summary: one sentence describing the design language's mood.\n\
- systemMd: a Markdown design system doc with 9 sections (theme & mood, color & roles, typography, \
spacing & grid, layout & responsive, component styles, elevation & depth, voice & tone, do's & don'ts).\n\
- tokens: an object of CSS custom properties. Fill EVERY key from this core vocabulary with a \
concrete value (colors as hex, sizes as px, fonts as font-family stacks): {vocab}. You MAY ALSO \
include any of these extended tokens when the source clearly implies them: {ext}\n\n\
{label}:\n{material}",
        source = source_label,
        vocab = TOKEN_VOCAB,
        ext = TOKEN_VOCAB_EXT,
        label = source_label.to_uppercase(),
        material = truncate(material, MATERIAL_CHARS),
    )
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        match s.char_indices().nth(max) {
            Some((i, _)) => &s[..i],
            None => s,
        }
    }
}

async fn run_extract(source_label: &str, material: &str) -> Result<ExtractedSystem> {
    if material.trim().is_empty() {
        anyhow::bail!("nothing to extract from");
    }
    let prompt = build_prompt(source_label, material);
    let config = crate::config::cached_config();
    let (agent, _model) = crate::recap::report::build_analysis_agent(&config).await?;
    // 4096：容纳完整 9 段 systemMd + 整套（核心 + 扩展）token 的 JSON，避免截断。
    let res = agent.side_query(&prompt, 4096).await?;
    parse(&res.text)
}

fn parse(text: &str) -> Result<ExtractedSystem> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str::<ExtractedSystem>(t) {
        return Ok(v);
    }
    if let (Some(a), Some(b)) = (t.find('{'), t.rfind('}')) {
        if b > a {
            if let Ok(v) = serde_json::from_str::<ExtractedSystem>(&t[a..=b]) {
                return Ok(v);
            }
        }
    }
    anyhow::bail!("could not parse extracted system JSON from model output")
}

/// 从一句话描述提取。
pub async fn from_brief(brief: &str) -> Result<ExtractedSystem> {
    run_extract("brand brief", brief).await
}

/// 设计方向候选（无品牌 brief 时的选择器，见 design-space.md §11.2）。
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Direction {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub tokens: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct DirectionsWrap {
    #[serde(default)]
    directions: Vec<Direction>,
}

/// 为一句话 brief 提 N 个不同气质的设计方向候选（不落盘，供用户/模型挑选）。
pub async fn propose_directions(brief: &str, n: usize) -> Result<Vec<Direction>> {
    let n = n.clamp(2, 6);
    let prompt = format!(
        "Propose {n} DISTINCT design directions for the brief below. Each should feel like a \
different brand personality (e.g. minimal, editorial, playful, corporate). Return ONLY a JSON \
object {{\"directions\":[...]}} where each item has: name (short label), summary (one sentence), \
tokens (an object using these CSS custom properties, concrete values — hex colors, px sizes, \
font stacks): {vocab}\n\nBRIEF:\n{brief}",
        n = n,
        vocab = TOKEN_VOCAB,
        brief = truncate(brief, 4000),
    );
    let config = crate::config::cached_config();
    let (agent, _model) = crate::recap::report::build_analysis_agent(&config).await?;
    let res = agent.side_query(&prompt, 2000).await?;
    let t = res.text.trim();
    let wrap: DirectionsWrap = serde_json::from_str(t)
        .or_else(|_| {
            let (a, b) = (t.find('{'), t.rfind('}'));
            match (a, b) {
                (Some(a), Some(b)) if b > a => serde_json::from_str(&t[a..=b]),
                _ => serde_json::from_str(t),
            }
        })
        .context("could not parse directions JSON")?;
    Ok(wrap.directions)
}

/// 从 URL 提取：抓**原始 HTML**（含 `<style>`/inline style，不走 Readability 清洗）
/// 后交 LLM 归纳。出站必过 SSRF（红线）。
pub async fn from_url(url: &str) -> Result<ExtractedSystem> {
    let html = fetch_raw_html(url).await?;
    if html.trim().is_empty() {
        anyhow::bail!("fetched empty page from {url}");
    }
    run_extract("web page raw HTML (with inline styles)", &html).await
}

/// 从 `Content-Type` header / `<meta charset>` 探测编码并正确解码（非 UTF-8 页——GBK /
/// Shift-JIS 等——不再 mojibake）；探测失败回退 UTF-8。
fn decode_html(bytes: &[u8], content_type: Option<&str>) -> String {
    // 1) Content-Type: text/html; charset=gbk
    let from_header = content_type.and_then(|ct| {
        ct.to_ascii_lowercase()
            .split("charset=")
            .nth(1)
            .map(|s| s.trim().trim_matches('"').trim().to_string())
    });
    // 2) <meta charset="..."> / <meta http-equiv content="...charset=..."> 在首段字节里嗅探。
    let from_meta = || {
        let head = &bytes[..bytes.len().min(4096)];
        let ascii = String::from_utf8_lossy(head).to_ascii_lowercase();
        ascii
            .find("charset=")
            .map(|i| &ascii[i + "charset=".len()..])
            .map(|rest| {
                rest.trim_start_matches(['"', '\'', ' '])
                    .split(['"', '\'', ' ', '/', '>', ';'])
                    .next()
                    .unwrap_or("")
                    .to_string()
            })
    };
    let label = from_header.or_else(from_meta).unwrap_or_default();
    let enc = encoding_rs::Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::UTF_8);
    let (cow, _, _) = enc.decode(bytes);
    cow.into_owned()
}

/// 抓取页面**原始 HTML**（不做正文抽取）。复用 web_fetch 的 SSRF + 浏览器头 + 代理
/// + 防 DNS-rebinding 重定向策略。上限 2MB（配合 charset 解码 + 更大提取窗口）。
async fn fetch_raw_html(url: &str) -> Result<String> {
    use futures_util::StreamExt;

    const MAX_BYTES: usize = 2 * 1024 * 1024;
    let ssrf_cfg = crate::config::cached_config().ssrf.clone();
    let policy = ssrf_cfg.web_fetch();
    let trusted = ssrf_cfg.trusted_hosts.clone();
    let parsed = crate::security::ssrf::check_url(url, policy, &trusted).await?;

    let redirect_hosts = trusted.clone();
    let redirect_policy = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= 5 {
            return attempt.error("too many redirects");
        }
        if let Some(host) = attempt.url().host_str() {
            if crate::security::ssrf::check_host_blocking_sync(host, policy, &redirect_hosts) {
                return attempt.stop();
            }
        }
        attempt.follow()
    });

    let client = crate::provider::apply_proxy(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .redirect(redirect_policy),
    )
    .build()
    .map_err(|e| anyhow::anyhow!("http client error: {e}"))?;

    let rb = crate::tools::web_fetch_common::apply_browser_headers(client.get(parsed));
    let resp = rb
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("fetch failed: {e}"))?;
    if !resp.status().is_success() {
        anyhow::bail!("fetch failed with status {}", resp.status().as_u16());
    }
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let mut bytes = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow::anyhow!("stream error: {e}"))?;
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_BYTES {
            bytes.truncate(MAX_BYTES);
            break;
        }
    }
    Ok(decode_html(&bytes, content_type.as_deref()))
}

/// 从本地代码库提取：读样本样式文件后交 LLM 归纳。
pub async fn from_codebase(dir: &Path) -> Result<ExtractedSystem> {
    let sample = collect_style_samples(dir)
        .with_context(|| format!("failed to read codebase at {}", dir.display()))?;
    if sample.trim().is_empty() {
        anyhow::bail!(
            "no style files (css / tailwind config / theme) found under {}",
            dir.display()
        );
    }
    run_extract("codebase style files", &sample).await
}

// ── Figma 导入（D2 网络通道，**owner 平面专属**：需 Figma 访问令牌，凭据不进模型面）─────────

/// 从 Figma 文件 URL 或 file key 解析出 file key。
/// 支持 `figma.com/file/{key}/…` / `figma.com/design/{key}/…` / 裸 key。
fn parse_figma_key(input: &str) -> Result<String> {
    let s = input.trim();
    if s.is_empty() {
        anyhow::bail!("Figma file URL or key is required");
    }
    // 裸 key（Figma key 是 [A-Za-z0-9]+）。
    if !s.contains('/') && !s.contains(':') && !s.contains('.') {
        return Ok(s.to_string());
    }
    for marker in ["/file/", "/design/"] {
        if let Some(i) = s.find(marker) {
            let key: String = s[i + marker.len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            if !key.is_empty() {
                return Ok(key);
            }
        }
    }
    anyhow::bail!("could not parse a Figma file key from '{input}'")
}

/// 对 api.figma.com 发起鉴权 GET（SSRF 校验 + `X-Figma-Token` header），返回解析后 JSON。
async fn figma_get(url: &str, token: &str) -> Result<serde_json::Value> {
    use futures_util::StreamExt;
    const MAX_BYTES: usize = 12 * 1024 * 1024; // Figma 文件 JSON 可能较大

    let ssrf_cfg = crate::config::cached_config().ssrf.clone();
    let policy = ssrf_cfg.web_fetch();
    let trusted = ssrf_cfg.trusted_hosts.clone();
    let parsed = crate::security::ssrf::check_url(url, policy, &trusted).await?;

    let client = crate::provider::apply_proxy(
        reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)),
    )
    .build()
    .map_err(|e| anyhow::anyhow!("http client error: {e}"))?;

    let resp = client
        .get(parsed)
        .header("X-Figma-Token", token)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Figma request failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let hint = match status.as_u16() {
            401 | 403 => " (check the token has file_read scope and access to this file)",
            404 => " (file not found — check the URL / key)",
            429 => " (rate limited — retry later)",
            _ => "",
        };
        anyhow::bail!("Figma API returned {}{hint}", status.as_u16());
    }
    let mut bytes = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow::anyhow!("stream error: {e}"))?;
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_BYTES {
            anyhow::bail!("Figma response exceeds {} MB cap", MAX_BYTES / (1024 * 1024));
        }
    }
    serde_json::from_slice(&bytes).map_err(|e| anyhow::anyhow!("Figma JSON parse error: {e}"))
}

/// Figma color（`{r,g,b,a}` 0..1 浮点）→ `#rrggbb[aa]`。
fn figma_color_hex(c: &serde_json::Value) -> Option<String> {
    let (r, g, b) = (
        c.get("r")?.as_f64()?,
        c.get("g")?.as_f64()?,
        c.get("b")?.as_f64()?,
    );
    let a = c.get("a").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let to = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    if a >= 0.999 {
        Some(format!("#{:02x}{:02x}{:02x}", to(r), to(g), to(b)))
    } else {
        Some(format!("#{:02x}{:02x}{:02x}{:02x}", to(r), to(g), to(b), to(a)))
    }
}

/// 从「已发布 styles + 其节点值」组装可读 material（颜色 hex / 文字排印 / 阴影）。
fn material_from_styles(styles: &serde_json::Value, nodes: &serde_json::Value) -> String {
    let (mut colors, mut texts, mut effects) = (Vec::new(), Vec::new(), Vec::new());
    let empty = Vec::new();
    for st in styles["meta"]["styles"].as_array().unwrap_or(&empty) {
        let name = st["name"].as_str().unwrap_or("");
        let node_id = st["node_id"].as_str().unwrap_or("");
        let doc = &nodes["nodes"][node_id]["document"];
        match st["style_type"].as_str().unwrap_or("") {
            "FILL" => {
                if let Some(hex) = doc["fills"]
                    .as_array()
                    .and_then(|f| f.iter().find(|p| p["type"] == "SOLID"))
                    .and_then(|p| figma_color_hex(&p["color"]))
                {
                    colors.push(format!("- '{name}': {hex}"));
                }
            }
            "TEXT" => {
                let s = &doc["style"];
                let fam = s["fontFamily"].as_str().unwrap_or("");
                let size = s["fontSize"].as_f64().unwrap_or(0.0);
                let weight = s["fontWeight"].as_f64().unwrap_or(0.0);
                if !fam.is_empty() || size > 0.0 {
                    texts.push(format!("- '{name}': {fam} {size}px weight {weight}"));
                }
            }
            "EFFECT" => {
                if let Some(eff) = doc["effects"]
                    .as_array()
                    .and_then(|e| e.iter().find(|x| x["type"] == "DROP_SHADOW"))
                {
                    let x = eff["offset"]["x"].as_f64().unwrap_or(0.0);
                    let y = eff["offset"]["y"].as_f64().unwrap_or(0.0);
                    let radius = eff["radius"].as_f64().unwrap_or(0.0);
                    let col = figma_color_hex(&eff["color"]).unwrap_or_default();
                    effects.push(format!("- '{name}': {x}px {y}px blur {radius}px {col}"));
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    if !colors.is_empty() {
        out.push(format!("Colors:\n{}", colors.join("\n")));
    }
    if !texts.is_empty() {
        out.push(format!("Text styles:\n{}", texts.join("\n")));
    }
    if !effects.is_empty() {
        out.push(format!("Effects (shadows):\n{}", effects.join("\n")));
    }
    out.join("\n\n")
}

/// 无已发布 styles 时的回退：遍历文档树采集去重的 SOLID 填充色（有界，防超大文件）。
fn material_from_document(doc: &serde_json::Value, cap: usize) -> String {
    let mut seen = std::collections::BTreeSet::new();
    let mut stack = vec![doc];
    let mut visited = 0usize;
    while let Some(node) = stack.pop() {
        if seen.len() >= cap || visited > 20000 {
            break;
        }
        visited += 1;
        if let Some(fills) = node.get("fills").and_then(|f| f.as_array()) {
            for p in fills {
                if p["type"] == "SOLID" {
                    if let Some(hex) = figma_color_hex(&p["color"]) {
                        seen.insert(hex);
                    }
                }
            }
        }
        if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
            for ch in children {
                stack.push(ch);
            }
        }
    }
    if seen.is_empty() {
        return String::new();
    }
    let colors: Vec<String> = seen.iter().map(|h| format!("- {h}")).collect();
    format!("Colors sampled from the document:\n{}", colors.join("\n"))
}

/// 从 **Figma 文件**提取品牌设计系统（D2 网络通道，**owner 平面专属**）。优先读已发布的
/// color/text/effect styles；无则回退采样文档 SOLID 填充色。汇成 material 后交 LLM 蒸馏成
/// 完整 9 段设计契约 + token（与 from_url / from_codebase 同管线）。
pub async fn from_figma(url_or_key: &str, token: &str) -> Result<ExtractedSystem> {
    let token = token.trim();
    if token.is_empty() {
        anyhow::bail!("Figma access token is required");
    }
    let key = parse_figma_key(url_or_key)?;

    // 1) 已发布 styles + 其节点值。
    let styles = figma_get(&format!("https://api.figma.com/v1/files/{key}/styles"), token).await?;
    let node_ids: Vec<String> = styles["meta"]["styles"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s["node_id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let mut material = String::new();
    if !node_ids.is_empty() {
        // Figma nodes 端点对 ids 数量有实际上限，取前 200 个足够覆盖一套设计系统。
        let ids: Vec<String> = node_ids.into_iter().take(200).collect();
        let nodes = figma_get(
            &format!("https://api.figma.com/v1/files/{key}/nodes?ids={}", ids.join(",")),
            token,
        )
        .await?;
        material = material_from_styles(&styles, &nodes);
    }

    // 2) 无已发布 styles → 回退采样文档填充色。
    if material.trim().is_empty() {
        let file = figma_get(&format!("https://api.figma.com/v1/files/{key}?depth=4"), token).await?;
        material = material_from_document(&file["document"], 60);
    }

    if material.trim().is_empty() {
        anyhow::bail!(
            "no published color/text/effect styles or filled layers found in this Figma file — publish your styles or ensure the file has colored layers"
        );
    }
    run_extract(
        "Figma file design styles (colors, typography, effects)",
        &material,
    )
    .await
}

/// 从**截图 / 设计图**提取（D2 视觉通道）。读本地图片文件 → 视觉模型分析 → 归纳
/// 品牌设计契约。走 design 层自包含视觉调用（不改主对话链路），支持 Anthropic /
/// OpenAI-Chat 两种格式的 vision 模型。
pub async fn from_image(path: &Path) -> Result<ExtractedSystem> {
    // Size cap (config `design.maxExtractImageMb`, default 24, `0` = unlimited).
    // Checked via metadata *before* reading so an oversized file never loads.
    let limit_mb = crate::config::cached_config().design.max_extract_image_mb;
    if limit_mb > 0 {
        let meta = std::fs::metadata(path)
            .with_context(|| format!("failed to stat image {}", path.display()))?;
        let max_bytes = (limit_mb as u64) * 1024 * 1024;
        if meta.len() > max_bytes {
            anyhow::bail!(
                "image is {} MiB, over the {} MB extraction limit (raise it in Settings → Tools → Design Space)",
                meta.len() / (1024 * 1024),
                limit_mb
            );
        }
    }
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read image {}", path.display()))?;
    if bytes.is_empty() {
        anyhow::bail!("image file is empty");
    }
    let mime = sniff_image_mime(&bytes);
    // 上传前按 vision provider 友好尺寸降采样 + 重压缩：本地闸只挡 OOM（默认 24MB），
    // 但原图 base64 后常超 provider 单图上限（如 Anthropic ~5MB / 1568px），会被 API 拒。
    let (bytes, mime) = downscale_for_vision(bytes, mime);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let prompt = build_prompt(
        "screenshot/design image",
        "(the design to analyze is provided as the attached image)",
    );
    let text = super::vision::vision_extract(&prompt, mime, &b64).await?;
    parse(&text)
}

/// 把参考图（base64）经 vision 模型**描述成详细重建 brief**，供「照着这张图生成匹配 `{kind}`
/// 产物」。与 [`from_image`]（图→设计系统 token）区别：这里产出可直接喂生成管线的重建指令
/// （布局 / 逐字文案 / 配色 / 字体 / 组件），生成一个视觉高度匹配的可交付产物。
pub async fn describe_reference_image(
    b64: &str,
    kind: super::renderer::ArtifactKind,
) -> Result<String> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .context("invalid reference image base64")?;
    if raw.is_empty() {
        anyhow::bail!("reference image is empty");
    }
    let limit_mb = crate::config::cached_config().design.max_extract_image_mb;
    if limit_mb > 0 && raw.len() as u64 > (limit_mb as u64) * 1024 * 1024 {
        anyhow::bail!(
            "reference image is over the {} MB limit (raise it in Settings → Tools → Design Space)",
            limit_mb
        );
    }
    let mime = sniff_image_mime(&raw); // 以魔数为准
    let (bytes, mime) = downscale_for_vision(raw, mime);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let prompt = format!(
        "你是资深产品设计师。仔细观察这张参考设计图，产出一份**足够详细、可据以从零重建**的设计说明，\
用于生成一个视觉上高度匹配的 **{kind}** 设计产物。请覆盖：整体布局与分区结构；每个区块的**真实可见\
文案**（逐字照抄图中文字，绝不用占位）；配色（主色 / 辅色 / 背景 / 文字色，尽量给近似色值）；字体风格\
与层级；间距与密度；关键组件（按钮 / 卡片 / 导航 / 表单等）及其样式；图形 / 插画 / 图标（生成时用内联 \
SVG 或 CSS 近似、无外链）。只输出这份重建说明本身，不寒暄、不加代码围栏。",
        kind = kind.as_str(),
    );
    super::vision::vision_extract(&prompt, mime, &b64).await
}

/// 把过大 / 过重的图缩到 vision provider 友好尺寸（长边 ≤ 1568px）并重编码 JPEG(q82)。
/// 任何解码 / 编码失败都**回退原图原 mime**（绝不阻断提取）。
fn downscale_for_vision(bytes: Vec<u8>, mime: &'static str) -> (Vec<u8>, &'static str) {
    const MAX_EDGE: u32 = 1568;
    const TARGET_BYTES: usize = 4 * 1024 * 1024;
    let img = match image::load_from_memory(&bytes) {
        Ok(i) => i,
        Err(_) => return (bytes, mime),
    };
    let (w, h) = (img.width(), img.height());
    if w.max(h) <= MAX_EDGE && bytes.len() <= TARGET_BYTES {
        return (bytes, mime);
    }
    // thumbnail 保持宽高比、快速降采样到框内。
    let resized = if w.max(h) > MAX_EDGE {
        img.thumbnail(MAX_EDGE, MAX_EDGE)
    } else {
        img
    };
    // JPEG 不支持 alpha：含透明通道的图先**合成到白底**，否则 to_rgb8 直接截通道会让透明区
    // 露出底层 RGB（常为黑）→ 设计图透明处变黑块、误导 vision 归纳配色。
    let rgb = if resized.color().has_alpha() {
        let rgba = resized.to_rgba8();
        let mut flat = image::RgbImage::new(rgba.width(), rgba.height());
        for (x, y, p) in rgba.enumerate_pixels() {
            let a = p[3] as u32;
            let over = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
            flat.put_pixel(x, y, image::Rgb([over(p[0]), over(p[1]), over(p[2])]));
        }
        flat
    } else {
        resized.to_rgb8()
    };
    let mut buf = Vec::new();
    let ok = {
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 82);
        enc.encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .is_ok()
    };
    if ok && !buf.is_empty() {
        (buf, "image/jpeg")
    } else {
        (bytes, mime)
    }
}

/// 从一份 **DESIGN.md** 文本导入设计系统（互通格式）：抽取显式 `--ds-*` token；足量
/// （≥4）则确定性直用（零 LLM 成本），不足则用 LLM 从正文合成。**始终保留原 DESIGN.md
/// 正文**（不改写用户的 prose）。
pub async fn from_design_md(md: &str) -> Result<ExtractedSystem> {
    if md.trim().is_empty() {
        anyhow::bail!("empty DESIGN.md");
    }
    let tokens = super::design_md::extract_tokens(md);
    let summary =
        super::design_md::extract_summary(md).unwrap_or_else(|| "导入的设计系统".to_string());
    let system_md = md.trim().to_string();
    if tokens.len() >= 4 {
        Ok(ExtractedSystem {
            summary,
            system_md,
            tokens,
        })
    } else {
        // token 不足 → LLM 从正文合成 token，但保留原 DESIGN.md 正文。
        let synth = from_brief(md).await?;
        Ok(ExtractedSystem {
            summary,
            system_md,
            tokens: synth.tokens,
        })
    }
}

/// 从图片魔数嗅探 mime（默认 png）。
fn sniff_image_mime(b: &[u8]) -> &'static str {
    if b.len() >= 3 && b[0] == 0xFF && b[1] == 0xD8 && b[2] == 0xFF {
        "image/jpeg"
    } else if b.len() >= 8 && &b[0..8] == b"\x89PNG\r\n\x1a\n" {
        "image/png"
    } else if b.len() >= 6 && (&b[0..6] == b"GIF87a" || &b[0..6] == b"GIF89a") {
        "image/gif"
    } else if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/png"
    }
}

/// 采集样式样本：CSS / tailwind config / theme 文件内容（有界深度/数量/大小）。
fn collect_style_samples(root: &Path) -> Result<String> {
    const MAX_FILES: usize = 40;
    const MAX_TOTAL: usize = 40000;
    const MAX_DEPTH: usize = 5;
    let mut out = String::new();
    let mut count = 0usize;
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_DEPTH || count >= MAX_FILES || out.len() >= MAX_TOTAL {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            // 跳过依赖 / 构建目录。
            if path.is_dir() {
                if matches!(
                    name.as_str(),
                    "node_modules" | ".git" | "dist" | "build" | "target" | ".next" | "vendor"
                ) {
                    continue;
                }
                stack.push((path, depth + 1));
                continue;
            }
            let lower = name.to_ascii_lowercase();
            let is_style = lower.ends_with(".css")
                || lower.ends_with(".scss")
                || lower.ends_with(".less")
                || lower.ends_with(".styl")
                || lower.starts_with("tailwind.config")
                || lower == "design.md"
                // 设计 token / 主题 / CSS-in-JS 文件：按文件名相关度匹配，避免读整棵源码树。
                || ((lower.contains("theme")
                    || lower.contains("token")
                    || lower.contains("palette")
                    || lower.contains("colors")
                    || lower.contains("design-system"))
                    && (lower.ends_with(".ts")
                        || lower.ends_with(".tsx")
                        || lower.ends_with(".js")
                        || lower.ends_with(".jsx")
                        || lower.ends_with(".mjs")
                        || lower.ends_with(".cjs")
                        || lower.ends_with(".json")));
            if !is_style {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                let take = content.len().min(MAX_TOTAL.saturating_sub(out.len()));
                out.push_str(&format!("\n/* --- {name} --- */\n"));
                out.push_str(truncate(&content, take));
                count += 1;
                if count >= MAX_FILES || out.len() >= MAX_TOTAL {
                    break;
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figma_key_parsing() {
        assert_eq!(parse_figma_key("ABC123def").unwrap(), "ABC123def");
        assert_eq!(
            parse_figma_key("https://www.figma.com/file/ABC123/My-Design?node-id=1-2").unwrap(),
            "ABC123"
        );
        assert_eq!(
            parse_figma_key("https://figma.com/design/XYZ789/Brand").unwrap(),
            "XYZ789"
        );
        assert!(parse_figma_key("").is_err());
        assert!(parse_figma_key("https://figma.com/community/foo").is_err());
    }

    #[test]
    fn figma_color_to_hex() {
        let c = serde_json::json!({ "r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0 });
        assert_eq!(figma_color_hex(&c).as_deref(), Some("#000000"));
        let c = serde_json::json!({ "r": 1.0, "g": 1.0, "b": 1.0 });
        assert_eq!(figma_color_hex(&c).as_deref(), Some("#ffffff"));
        // 半透明 → 8 位。
        let c = serde_json::json!({ "r": 0.5, "g": 0.5, "b": 0.5, "a": 0.5 });
        assert_eq!(figma_color_hex(&c).as_deref(), Some("#80808080"));
        // 缺分量 → None（不 panic）。
        assert_eq!(figma_color_hex(&serde_json::json!({ "r": 0.1 })), None);
    }

    #[test]
    fn figma_material_from_published_styles() {
        let styles = serde_json::json!({ "meta": { "styles": [
            { "name": "Primary", "node_id": "1:2", "style_type": "FILL" },
            { "name": "Heading", "node_id": "1:3", "style_type": "TEXT" },
            { "name": "Card", "node_id": "1:4", "style_type": "EFFECT" }
        ]}});
        let nodes = serde_json::json!({ "nodes": {
            "1:2": { "document": { "fills": [ { "type": "SOLID", "color": { "r": 0.1, "g": 0.2, "b": 0.9, "a": 1.0 } } ] } },
            "1:3": { "document": { "style": { "fontFamily": "Inter", "fontSize": 24.0, "fontWeight": 700.0 } } },
            "1:4": { "document": { "effects": [ { "type": "DROP_SHADOW", "offset": { "x": 0.0, "y": 2.0 }, "radius": 8.0, "color": { "r": 0.0, "g": 0.0, "b": 0.0, "a": 0.1 } } ] } }
        }});
        let m = material_from_styles(&styles, &nodes);
        assert!(m.contains("'Primary': #1a33e6"), "color: {m}");
        assert!(m.contains("'Heading': Inter 24px weight 700"), "text: {m}");
        assert!(m.contains("'Card':") && m.contains("blur 8px"), "effect: {m}");
    }

    #[test]
    fn figma_material_missing_node_is_skipped() {
        // node_id 在 styles 里但 nodes 响应缺失 → 跳过、不 panic。
        let styles = serde_json::json!({ "meta": { "styles": [
            { "name": "Ghost", "node_id": "9:9", "style_type": "FILL" }
        ]}});
        let nodes = serde_json::json!({ "nodes": {} });
        assert_eq!(material_from_styles(&styles, &nodes), "");
    }

    #[test]
    fn figma_document_fallback_dedups_colors() {
        let doc = serde_json::json!({
            "fills": [ { "type": "SOLID", "color": { "r": 1.0, "g": 0.0, "b": 0.0 } } ],
            "children": [
                { "fills": [ { "type": "SOLID", "color": { "r": 1.0, "g": 0.0, "b": 0.0 } } ] },
                { "fills": [ { "type": "SOLID", "color": { "r": 0.0, "g": 1.0, "b": 0.0 } } ],
                  "children": [ { "fills": [ { "type": "GRADIENT_LINEAR" } ] } ] }
            ]
        });
        let m = material_from_document(&doc, 60);
        assert!(m.contains("#ff0000") && m.contains("#00ff00"));
        // 去重：#ff0000 只出现一次。
        assert_eq!(m.matches("#ff0000").count(), 1);
    }

    #[test]
    fn parse_fenced() {
        let j = "```json\n{\"summary\":\"clean\",\"systemMd\":\"# X\",\"tokens\":{\"--ds-color-primary\":\"#111\"}}\n```";
        let r = parse(j).unwrap();
        assert_eq!(r.summary, "clean");
        assert_eq!(r.tokens.get("--ds-color-primary").unwrap(), "#111");
    }

    #[test]
    fn collect_samples_reads_css() -> Result<()> {
        let tmp = std::env::temp_dir().join(format!("ds-extract-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        std::fs::write(tmp.join("theme.css"), ":root{--brand:#123456}").unwrap();
        std::fs::create_dir_all(tmp.join("node_modules")).unwrap();
        std::fs::write(
            tmp.join("node_modules").join("junk.css"),
            "should be skipped",
        )
        .unwrap();
        let s = collect_style_samples(&tmp)?;
        assert!(s.contains("--brand:#123456"));
        assert!(!s.contains("should be skipped"));
        let _ = std::fs::remove_dir_all(&tmp);
        Ok(())
    }
}
