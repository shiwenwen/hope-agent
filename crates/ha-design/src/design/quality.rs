//! 确定性视觉回归：固定视口、内容寻址截图、像素差异与静态 DOM/无障碍检查。
//!
//! 基线是产物目录下的本地真相源；模型视觉评审不参与通过/失败裁决。

use anyhow::{Context, Result};
use ha_browser::browser::backend::{BrowserBackend, ImageFormat, ScreenshotParams, SnapshotFormat};
use ha_browser::browser::cdp_backend::CdpBackend;
use ha_core::platform::write_atomic;
use image::{DynamicImage, GenericImageView};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MANIFEST_VERSION: u32 = 1;
const MAX_PNG_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct QualityViewport {
    pub width: u32,
    pub height: u32,
}

pub const FIXED_VIEWPORTS: [QualityViewport; 3] = [
    QualityViewport {
        width: 1440,
        height: 900,
    },
    QualityViewport {
        width: 768,
        height: 1024,
    },
    QualityViewport {
        width: 390,
        height: 844,
    },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineEntry {
    pub viewport: QualityViewport,
    pub image_hash: String,
    pub artifact_hash: String,
    pub accepted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityManifest {
    pub version: u32,
    pub artifact_id: String,
    #[serde(default)]
    pub baselines: BTreeMap<String, BaselineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StaticFinding {
    pub code: String,
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewportDiff {
    pub viewport: QualityViewport,
    pub current_hash: String,
    pub baseline_hash: Option<String>,
    pub changed_ratio: Option<f64>,
    pub mean_delta: Option<f64>,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityRun {
    pub artifact_id: String,
    pub artifact_hash: String,
    pub diffs: Vec<ViewportDiff>,
    pub findings: Vec<StaticFinding>,
    pub deterministic_passed: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptBaselineInput {
    pub artifact_id: String,
    pub expected_artifact_hash: String,
}

fn key(v: QualityViewport) -> String {
    format!("{}x{}", v.width, v.height)
}

fn quality_dir(project_id: &str, artifact_id: &str) -> Result<PathBuf> {
    Ok(ha_core::paths::design_artifact_dir(project_id, artifact_id)?.join("quality"))
}

fn load_manifest(dir: &Path, artifact_id: &str) -> Result<QualityManifest> {
    let path = dir.join("manifest.json");
    match std::fs::read(&path) {
        Ok(bytes) => {
            let parsed: QualityManifest = serde_json::from_slice(&bytes)
                .with_context(|| format!("parse {}", path.display()))?;
            if parsed.version != MANIFEST_VERSION || parsed.artifact_id != artifact_id {
                anyhow::bail!("quality manifest identity/version mismatch");
            }
            Ok(parsed)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(QualityManifest {
            version: MANIFEST_VERSION,
            artifact_id: artifact_id.to_string(),
            baselines: BTreeMap::new(),
        }),
        Err(e) => Err(e).context("read quality manifest"),
    }
}

fn artifact_hash(artifact_id: &str) -> Result<String> {
    let source = super::service::get_artifact_source_for_agent(artifact_id)?
        .with_context(|| format!("artifact not found: {artifact_id}"))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(source.body.as_bytes());
    hasher.update(&[0]);
    hasher.update(source.css.as_bytes());
    hasher.update(&[0]);
    hasher.update(source.js.as_bytes());
    Ok(hasher.finalize().to_hex().to_string())
}

fn static_findings(html: &str) -> Vec<StaticFinding> {
    let doc = scraper::Html::parse_document(html);
    let mut out = Vec::new();
    let img = scraper::Selector::parse("img:not([alt])").expect("constant selector");
    if doc.select(&img).next().is_some() {
        out.push(StaticFinding {
            code: "image-missing-alt".into(),
            severity: "error".into(),
            message: "存在缺少 alt 的图片".into(),
        });
    }
    let html_tag = scraper::Selector::parse("html").expect("constant selector");
    if doc
        .select(&html_tag)
        .next()
        .and_then(|n| n.value().attr("lang"))
        .is_none()
    {
        out.push(StaticFinding {
            code: "document-missing-lang".into(),
            severity: "warning".into(),
            message: "文档未声明 lang".into(),
        });
    }
    let interactive = scraper::Selector::parse("button, a[href]").expect("constant selector");
    if doc.select(&interactive).any(|n| {
        n.text().collect::<String>().trim().is_empty()
            && n.value().attr("aria-label").is_none()
            && n.value().attr("title").is_none()
    }) {
        out.push(StaticFinding {
            code: "interactive-missing-name".into(),
            severity: "error".into(),
            message: "存在缺少可访问名称的按钮或链接".into(),
        });
    }
    out
}

fn compare_images(current: &DynamicImage, baseline: &DynamicImage) -> (f64, f64) {
    let (cw, ch) = current.dimensions();
    let (bw, bh) = baseline.dimensions();
    if (cw, ch) != (bw, bh) {
        return (1.0, 1.0);
    }
    let current = current.to_rgba8();
    let baseline = baseline.to_rgba8();
    let mut changed = 0_u64;
    let mut delta = 0_u64;
    for (a, b) in current.pixels().zip(baseline.pixels()) {
        let d: u64 =
            a.0.iter()
                .zip(b.0.iter())
                .map(|(x, y)| u64::from(x.abs_diff(*y)))
                .sum();
        delta += d;
        if d > 16 {
            changed += 1;
        }
    }
    let pixels = u64::from(cw) * u64::from(ch);
    (
        changed as f64 / pixels.max(1) as f64,
        delta as f64 / (pixels.max(1) * 4 * 255) as f64,
    )
}

async fn capture_all(artifact_id: &str) -> Result<Vec<(QualityViewport, Vec<u8>)>> {
    let artifact = super::service::get_artifact(artifact_id)?
        .with_context(|| format!("artifact not found: {artifact_id}"))?;
    let dir = ha_core::paths::design_artifact_dir(&artifact.project_id, &artifact.id)?;
    let url = url::Url::from_file_path(dir.join("index.html"))
        .map_err(|_| anyhow::anyhow!("artifact path cannot be represented as a file URL"))?
        .to_string();
    // Visual regression must never claim a tab from the user's attached
    // Chrome extension session. Direct CDP uses Hope's managed browser and a
    // disposable page, while still preserving any managed tab that was active.
    let backend: Arc<dyn BrowserBackend> = Arc::new(CdpBackend::new());
    let original_target = backend
        .status()
        .await
        .ok()
        .and_then(|status| status.active_target_id);
    let original_viewport = if let Some(target_id) = original_target.as_deref() {
        let _ = backend.select_page(target_id).await;
        backend
            .take_snapshot(SnapshotFormat::Role)
            .await
            .ok()
            .map(|snapshot| snapshot.viewport)
    } else {
        None
    };
    let tab = backend.new_page(Some(&url)).await?;
    let _ = backend.select_page(&tab.target_id).await;
    let result = async {
        if !tab.url.starts_with("file://") {
            backend.navigate(&url).await?;
        }
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let mut captures = Vec::with_capacity(FIXED_VIEWPORTS.len());
        for viewport in FIXED_VIEWPORTS {
            backend.resize(viewport.width, viewport.height).await?;
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            let bytes = backend
                .take_screenshot(ScreenshotParams {
                    format: ImageFormat::Png,
                    full_page: false,
                    ..Default::default()
                })
                .await?;
            if bytes.len() > MAX_PNG_BYTES {
                anyhow::bail!("captured PNG exceeds 32 MiB");
            }
            captures.push((viewport, bytes));
        }
        Ok::<_, anyhow::Error>(captures)
    }
    .await;
    let _ = backend.close_page(&tab.target_id).await;
    if let Some(target_id) = original_target.as_deref() {
        let _ = backend.select_page(target_id).await;
        if let Some((width, height)) = original_viewport {
            let _ = backend.resize(width, height).await;
        }
    }
    result
}

pub async fn run(artifact_id: &str) -> Result<QualityRun> {
    let artifact = super::service::get_artifact(artifact_id)?
        .with_context(|| format!("artifact not found: {artifact_id}"))?;
    let hash = artifact_hash(artifact_id)?;
    let dir = quality_dir(&artifact.project_id, artifact_id)?;
    let manifest = load_manifest(&dir, artifact_id)?;
    let html = std::fs::read_to_string(
        ha_core::paths::design_artifact_dir(&artifact.project_id, artifact_id)?.join("index.html"),
    )?;
    let findings = static_findings(&html);
    let captures = capture_all(artifact_id).await?;
    if artifact_hash(artifact_id)? != hash {
        anyhow::bail!("artifact changed during visual regression capture");
    }
    let mut diffs = Vec::with_capacity(captures.len());
    for (viewport, bytes) in captures {
        let current_hash = blake3::hash(&bytes).to_hex().to_string();
        let current = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)?;
        let baseline = manifest.baselines.get(&key(viewport));
        let (changed_ratio, mean_delta, passed) = if let Some(entry) = baseline {
            let path = dir
                .join("screenshots")
                .join(format!("{}.png", entry.image_hash));
            let previous = image::open(path)?;
            let (changed, delta) = compare_images(&current, &previous);
            (
                Some(changed),
                Some(delta),
                changed <= 0.001 && delta <= 0.002,
            )
        } else {
            (None, None, false)
        };
        diffs.push(ViewportDiff {
            viewport,
            current_hash,
            baseline_hash: baseline.map(|b| b.image_hash.clone()),
            changed_ratio,
            mean_delta,
            passed,
        });
    }
    let deterministic_passed =
        diffs.iter().all(|d| d.passed) && !findings.iter().any(|f| f.severity == "error");
    Ok(QualityRun {
        artifact_id: artifact_id.to_string(),
        artifact_hash: hash,
        diffs,
        findings,
        deterministic_passed,
    })
}

/// 显式 owner 操作：以期望源码哈希防止把陈旧画面接受为当前基线。
pub async fn accept(input: AcceptBaselineInput) -> Result<QualityManifest> {
    let artifact = super::service::get_artifact(&input.artifact_id)?
        .with_context(|| format!("artifact not found: {}", input.artifact_id))?;
    let actual = artifact_hash(&input.artifact_id)?;
    if actual != input.expected_artifact_hash {
        anyhow::bail!("stale baseline acceptance: artifact changed");
    }
    let dir = quality_dir(&artifact.project_id, &artifact.id)?;
    std::fs::create_dir_all(dir.join("screenshots"))?;
    let mut manifest = load_manifest(&dir, &artifact.id)?;
    let now = chrono::Utc::now().to_rfc3339();
    let captures = capture_all(&artifact.id).await?;
    if artifact_hash(&artifact.id)? != actual {
        anyhow::bail!("artifact changed during baseline capture");
    }
    for (viewport, bytes) in captures {
        let image_hash = blake3::hash(&bytes).to_hex().to_string();
        write_atomic(
            &dir.join("screenshots").join(format!("{image_hash}.png")),
            &bytes,
        )?;
        manifest.baselines.insert(
            key(viewport),
            BaselineEntry {
                viewport,
                image_hash,
                artifact_hash: actual.clone(),
                accepted_at: now.clone(),
            },
        );
    }
    let bytes = serde_json::to_vec_pretty(&manifest)?;
    write_atomic(&dir.join("manifest.json"), &bytes)?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_viewports_are_bounded_and_unique() {
        let mut keys = std::collections::BTreeSet::new();
        for viewport in FIXED_VIEWPORTS {
            assert!(viewport.width <= 1440 && viewport.height <= 1024);
            assert!(keys.insert(key(viewport)));
        }
    }

    #[test]
    fn static_rules_find_missing_names() {
        let findings = static_findings("<html><body><img><button></button></body></html>");
        assert!(findings.iter().any(|f| f.code == "image-missing-alt"));
        assert!(findings
            .iter()
            .any(|f| f.code == "interactive-missing-name"));
    }
}
