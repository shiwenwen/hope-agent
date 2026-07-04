//! 设计系统反向提取（D2 护城河）。
//!
//! 从**文本描述**或**本地代码库**（读现有 CSS / tailwind / theme 文件样本）反向生成
//! 品牌设计契约（`SYSTEM.md` + `tokens.json`）。"读本地工程提取设计系统" 是云端产品
//! 做不到的本地能力。截图 / URL 多模态提取列后续迭代。见 design-space.md §6.4。

use anyhow::{Context, Result};
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

/// 从本地代码库提取：读样本样式文件后交 LLM 归纳。
pub async fn from_codebase(dir: &Path) -> Result<ExtractedSystem> {
    let sample = collect_style_samples(dir)
        .with_context(|| format!("failed to read codebase at {}", dir.display()))?;
    if sample.trim().is_empty() {
        anyhow::bail!("no style files (css / tailwind config / theme) found under {}", dir.display());
    }
    run_extract("codebase style files", &sample).await
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
    fn collect_samples_reads_css(
    ) -> Result<()> {
        let tmp = std::env::temp_dir().join(format!("ds-extract-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        std::fs::write(tmp.join("theme.css"), ":root{--brand:#123456}").unwrap();
        std::fs::create_dir_all(tmp.join("node_modules")).unwrap();
        std::fs::write(tmp.join("node_modules").join("junk.css"), "should be skipped").unwrap();
        let s = collect_style_samples(&tmp)?;
        assert!(s.contains("--brand:#123456"));
        assert!(!s.contains("should be skipped"));
        let _ = std::fs::remove_dir_all(&tmp);
        Ok(())
    }
}
