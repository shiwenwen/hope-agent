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

const TOKEN_VOCAB: &str = "--ds-color-bg, --ds-color-fg, --ds-color-primary, --ds-color-secondary, \
--ds-color-accent, --ds-color-muted, --ds-color-border, --ds-color-success, --ds-color-warning, \
--ds-color-danger, --ds-font-sans, --ds-font-serif, --ds-font-mono, --ds-text-base, --ds-text-lg, \
--ds-text-xl, --ds-text-2xl, --ds-text-3xl, --ds-space-2, --ds-space-4, --ds-space-6, --ds-space-8, \
--ds-radius-md, --ds-radius-lg, --ds-shadow-md";

fn build_prompt(source_label: &str, material: &str) -> String {
    format!(
        "You are a brand designer distilling a reusable design system. Based on the {source} below, \
produce a cohesive brand design contract.\n\n\
Return ONLY a JSON object (no prose, no code fence) with keys:\n\
- summary: one sentence describing the design language's mood.\n\
- systemMd: a Markdown design system doc with 9 sections (theme & mood, color & roles, typography, \
spacing & grid, layout & responsive, component styles, elevation & depth, voice & tone, do's & don'ts).\n\
- tokens: an object of CSS custom properties using EXACTLY this vocabulary (fill every key with a \
concrete value; colors as hex, sizes as px, fonts as font-family stacks): {vocab}\n\n\
{label}:\n{material}",
        source = source_label,
        vocab = TOKEN_VOCAB,
        label = source_label.to_uppercase(),
        material = truncate(material, 16000),
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
    let res = agent.side_query(&prompt, 2000).await?;
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

/// 抓取页面**原始 HTML**（不做正文抽取）。复用 web_fetch 的 SSRF + 浏览器头 + 代理
/// + 防 DNS-rebinding 重定向策略。上限 512KB。
async fn fetch_raw_html(url: &str) -> Result<String> {
    use futures_util::StreamExt;

    const MAX_BYTES: usize = 512 * 1024;
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
    Ok(String::from_utf8_lossy(&bytes).into_owned())
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

/// 从**截图 / 设计图**提取（D2 视觉通道）。读本地图片文件 → 视觉模型分析 → 归纳
/// 品牌设计契约。走 design 层自包含视觉调用（不改主对话链路），支持 Anthropic /
/// OpenAI-Chat 两种格式的 vision 模型。
pub async fn from_image(path: &Path) -> Result<ExtractedSystem> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read image {}", path.display()))?;
    if bytes.is_empty() {
        anyhow::bail!("image file is empty");
    }
    let mime = sniff_image_mime(&bytes);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let prompt = build_prompt(
        "screenshot/design image",
        "(the design to analyze is provided as the attached image)",
    );
    let text = super::vision::vision_extract(&prompt, mime, &b64).await?;
    parse(&text)
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
    const MAX_FILES: usize = 20;
    const MAX_TOTAL: usize = 14000;
    const MAX_DEPTH: usize = 4;
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
            let is_style = name.ends_with(".css")
                || name.ends_with(".scss")
                || name.starts_with("tailwind.config")
                || name.starts_with("theme")
                || name == "DESIGN.md";
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
