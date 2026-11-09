//! 确定性视觉回归：固定视口、内容寻址截图、像素差异与静态 DOM/无障碍检查。
//!
//! 基线是产物目录下的本地真相源；模型视觉评审不参与通过/失败裁决。

use anyhow::{Context, Result};
use base64::Engine;
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
const MAX_TOTAL_PNG_BYTES: usize = 128 * 1024 * 1024;
const CAPTURE_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; media-src data: blob:; font-src data: blob:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; navigate-to 'none'";

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
    #[serde(default = "default_scenario_id")]
    pub scenario_id: String,
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
    pub scenario_id: String,
    pub scenario_title: String,
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
    pub scenarios_hash: String,
    pub diffs: Vec<ViewportDiff>,
    pub findings: Vec<StaticFinding>,
    pub deterministic_passed: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptBaselineInput {
    pub artifact_id: String,
    pub expected_artifact_hash: String,
    #[serde(default)]
    pub expected_scenarios_hash: Option<String>,
}

fn default_scenario_id() -> String {
    "default".into()
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

#[derive(Debug, Clone)]
struct CaptureCase {
    scenario: super::scenarios::DesignScenario,
    viewport: QualityViewport,
}

fn is_default_scenario(scenario: &super::scenarios::DesignScenario) -> bool {
    scenario.id == "default"
        && scenario.route == "/"
        && scenario.ready.is_none()
        && (scenario.state.is_null() || scenario.state.as_object().is_some_and(|s| s.is_empty()))
}

fn capture_plan(manifest: &super::scenarios::ScenariosManifest) -> Result<Vec<CaptureCase>> {
    super::scenarios::validate(manifest)?;
    let mut plan = Vec::new();
    for scenario in &manifest.scenarios {
        anyhow::ensure!(
            scenario.route == "/",
            "visual_scenarios_incomplete: only root-route state projection is supported"
        );
        anyhow::ensure!(is_default_scenario(scenario) || scenario.ready.is_some(),
            "visual_scenarios_incomplete: custom scenarios require a ready selector and expected text");
        let viewports = if scenario.viewport_ids.is_empty() {
            FIXED_VIEWPORTS.to_vec()
        } else {
            scenario
                .viewport_ids
                .iter()
                .map(|id| {
                    let v = &manifest.viewports[id];
                    QualityViewport {
                        width: v.width,
                        height: v.height,
                    }
                })
                .collect()
        };
        plan.extend(viewports.into_iter().map(|viewport| CaptureCase {
            scenario: scenario.clone(),
            viewport,
        }));
    }
    let identities: std::collections::BTreeSet<_> = plan.iter().map(capture_key).collect();
    anyhow::ensure!(
        identities.len() == plan.len(),
        "visual_scenarios_incomplete: duplicate scenario viewport dimensions"
    );
    Ok(plan)
}

fn capture_key(case: &CaptureCase) -> String {
    if is_default_scenario(&case.scenario) {
        return key(case.viewport);
    }
    let hash = blake3::hash(&serde_json::to_vec(&case.scenario).expect("serializable scenario"));
    format!(
        "{}@{}:{}",
        case.scenario.id,
        hash.to_hex(),
        key(case.viewport)
    )
}

fn ensure_capture_set(plan: &[CaptureCase], captures: &[(CaptureCase, Vec<u8>)]) -> Result<()> {
    anyhow::ensure!(
        plan.len() == captures.len() && !plan.is_empty(),
        "visual_scenarios_incomplete: capture set is incomplete"
    );
    let expected: std::collections::BTreeSet<_> = plan.iter().map(capture_key).collect();
    let actual: std::collections::BTreeSet<_> =
        captures.iter().map(|(case, _)| capture_key(case)).collect();
    anyhow::ensure!(
        expected == actual && actual.len() == captures.len(),
        "visual_scenarios_incomplete: capture identities do not match the plan"
    );
    Ok(())
}

fn ensure_scenarios_unchanged(
    current: &super::scenarios::ScenariosEnvelope,
    expected_hash: &str,
) -> Result<()> {
    capture_plan(&current.manifest)?;
    anyhow::ensure!(
        current.hash == expected_hash,
        "visual_scenarios_incomplete: scenarios changed during capture"
    );
    Ok(())
}

struct QualityRunPreparation {
    artifact_id: String,
    artifact_hash: String,
    dir: PathBuf,
    manifest: QualityManifest,
    findings: Vec<StaticFinding>,
    capture_html: String,
    scenarios_hash: String,
    plan: Vec<CaptureCase>,
}

struct BaselinePreparation {
    artifact_id: String,
    artifact_hash: String,
    dir: PathBuf,
    manifest: QualityManifest,
    capture_html: String,
    scenarios_hash: String,
    plan: Vec<CaptureCase>,
    _artifact_lifecycle_guard: std::fs::File,
}

fn artifact_capture_html(project_id: &str, artifact_id: &str) -> Result<String> {
    let dir = ha_core::paths::design_artifact_dir(project_id, artifact_id)?;
    std::fs::read_to_string(dir.join("index.html")).context("artifact has no rendered index.html")
}

fn opening_tag_end(serialized_element: &str, tag: &str) -> Result<usize> {
    let expected = format!("<{tag}");
    if !serialized_element
        .get(..expected.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(&expected))
        || !serialized_element
            .as_bytes()
            .get(expected.len())
            .is_some_and(|next| next.is_ascii_whitespace() || *next == b'>')
    {
        anyhow::bail!("artifact capture has an invalid serialized {tag} element");
    }
    let mut quote = None;
    for (offset, byte) in serialized_element.bytes().enumerate().skip(expected.len()) {
        match (quote, byte) {
            (None, b'\'' | b'\"') => quote = Some(byte),
            (Some(active), current) if active == current => quote = None,
            (None, b'>') => return Ok(offset + 1),
            _ => {}
        }
    }
    anyhow::bail!("artifact capture has an unterminated {tag} element")
}

fn inject_capture_csp(html: &str) -> Result<String> {
    // Select the parser-created document elements instead of searching source
    // text: comments or attribute values may contain a spoofed `<head>` before
    // the real element. Rebuilding only the parsed html/head/body structure also
    // gives the policy a deterministic position before every executable child.
    let document = scraper::Html::parse_document(html);
    let root = document.root_element();
    let head = root
        .child_elements()
        .find(|element| element.value().name() == "head")
        .ok_or_else(|| anyhow::anyhow!("artifact capture requires a head element"))?;
    let body = root
        .child_elements()
        .find(|element| element.value().name() == "body")
        .ok_or_else(|| anyhow::anyhow!("artifact capture requires a body element"))?;
    let root_html = root.html();
    let head_html = head.html();
    let body_html = body.html();
    let root_open_end = opening_tag_end(&root_html, "html")?;
    let head_open_end = opening_tag_end(&head_html, "head")?;
    let policy = format!("<meta http-equiv=\"Content-Security-Policy\" content=\"{CAPTURE_CSP}\">");
    let mut secured = String::with_capacity(root_html.len() + policy.len() + 15);
    secured.push_str("<!doctype html>");
    secured.push_str(&root_html[..root_open_end]);
    secured.push_str(&head_html[..head_open_end]);
    secured.push_str(&policy);
    secured.push_str(&head_html[head_open_end..]);
    secured.push_str(&body_html);
    secured.push_str("</html>");
    Ok(secured)
}

fn capture_wrapper_script(html: &str) -> Result<String> {
    let secured = inject_capture_csp(html)?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(secured.as_bytes());
    Ok(format!(
        r#"(() => {{
            document.documentElement.innerHTML = '<head><meta name="referrer" content="no-referrer"><style>html,body{{margin:0;width:100%;height:100%;overflow:hidden;background:white}}iframe{{display:block;width:100%;height:100%;border:0}}</style></head><body></body>';
            window.__dsQualityReady = false;
            const frame = document.createElement('iframe');
            frame.setAttribute('sandbox', 'allow-scripts');
            frame.setAttribute('referrerpolicy', 'no-referrer');
            frame.addEventListener('load', () => {{ window.__dsQualityReady = true; }}, {{ once: true }});
            const bytes = Uint8Array.from(atob('{encoded}'), c => c.charCodeAt(0));
            frame.srcdoc = new TextDecoder().decode(bytes);
            document.body.appendChild(frame);
            return frame.getAttribute('sandbox');
        }})()"#
    ))
}

fn prepare_quality_run(artifact_id: &str) -> Result<QualityRunPreparation> {
    let artifact = super::service::get_artifact(artifact_id)?
        .with_context(|| format!("artifact not found: {artifact_id}"))?;
    let artifact_hash = artifact_hash(artifact_id)?;
    let scenarios = super::scenarios::get(artifact_id)?;
    let plan = capture_plan(&scenarios.manifest)?;
    let dir = quality_dir(&artifact.project_id, artifact_id)?;
    let manifest = load_manifest(&dir, artifact_id)?;
    let html = artifact_capture_html(&artifact.project_id, artifact_id)?;
    Ok(QualityRunPreparation {
        artifact_id: artifact_id.to_string(),
        artifact_hash,
        dir,
        manifest,
        findings: static_findings(&html),
        capture_html: html,
        scenarios_hash: scenarios.hash,
        plan,
    })
}

fn finish_quality_run(
    prepared: QualityRunPreparation,
    captures: Vec<(CaptureCase, Vec<u8>)>,
) -> Result<QualityRun> {
    ensure_capture_set(&prepared.plan, &captures)?;
    if artifact_hash(&prepared.artifact_id)? != prepared.artifact_hash {
        anyhow::bail!("artifact changed during visual regression capture");
    }
    ensure_scenarios_unchanged(
        &super::scenarios::get(&prepared.artifact_id)?,
        &prepared.scenarios_hash,
    )?;
    let mut diffs = Vec::with_capacity(captures.len());
    for (case, bytes) in captures {
        let viewport = case.viewport;
        let current_hash = blake3::hash(&bytes).to_hex().to_string();
        let current = capture_image(&bytes, viewport)?;
        let baseline = prepared
            .manifest
            .baselines
            .get(&capture_key(&case))
            .filter(|entry| entry.viewport == viewport && entry.scenario_id == case.scenario.id);
        let (changed_ratio, mean_delta, passed) = if let Some(entry) = baseline {
            let path = prepared
                .dir
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
            scenario_id: case.scenario.id,
            scenario_title: case.scenario.title,
            viewport,
            current_hash,
            baseline_hash: baseline.map(|b| b.image_hash.clone()),
            changed_ratio,
            mean_delta,
            passed,
        });
    }
    let deterministic_passed =
        diffs.iter().all(|d| d.passed) && !prepared.findings.iter().any(|f| f.severity == "error");
    Ok(QualityRun {
        artifact_id: prepared.artifact_id,
        artifact_hash: prepared.artifact_hash,
        scenarios_hash: prepared.scenarios_hash,
        diffs,
        findings: prepared.findings,
        deterministic_passed,
    })
}

fn ensure_acceptance_scenarios(
    scenarios: &super::scenarios::ScenariosEnvelope,
    expected: Option<&str>,
) -> Result<()> {
    if let Some(expected) = expected {
        anyhow::ensure!(
            expected == scenarios.hash,
            "visual_scenarios_incomplete: reviewed scenarios changed before baseline acceptance"
        );
    } else {
        anyhow::ensure!(scenarios.manifest.viewports.is_empty()
            && scenarios.manifest.scenarios.len() == 1
            && is_default_scenario(&scenarios.manifest.scenarios[0])
            && scenarios.manifest.scenarios[0].viewport_ids.is_empty(),
            "visual_scenarios_incomplete: custom baseline acceptance requires expectedScenariosHash");
    }
    Ok(())
}

fn capture_image(bytes: &[u8], viewport: QualityViewport) -> Result<DynamicImage> {
    anyhow::ensure!(bytes.len() <= MAX_PNG_BYTES, "captured PNG exceeds 32 MiB");
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)?;
    anyhow::ensure!(
        image.dimensions() == (viewport.width, viewport.height),
        "captured PNG dimensions do not match the planned viewport"
    );
    Ok(image)
}

fn prepare_baseline(input: AcceptBaselineInput) -> Result<BaselinePreparation> {
    let candidate = super::service::get_artifact(&input.artifact_id)?
        .with_context(|| format!("artifact not found: {}", input.artifact_id))?;
    let artifact_lifecycle_guard =
        super::service::acquire_artifact_lifecycle_lock(&candidate.project_id, &candidate.id)?;
    // Deletion may have won before the lifecycle lock was acquired. Re-read
    // the durable identity under the lock before creating any quality path.
    let artifact = super::service::get_artifact(&input.artifact_id)?
        .with_context(|| format!("artifact not found: {}", input.artifact_id))?;
    if artifact.project_id != candidate.project_id {
        anyhow::bail!("artifact identity changed while accepting baseline");
    }
    let actual = artifact_hash(&input.artifact_id)?;
    if actual != input.expected_artifact_hash {
        anyhow::bail!("stale baseline acceptance: artifact changed");
    }
    let scenarios = super::scenarios::get(&artifact.id)?;
    let plan = capture_plan(&scenarios.manifest)?;
    ensure_acceptance_scenarios(&scenarios, input.expected_scenarios_hash.as_deref())?;
    let dir = quality_dir(&artifact.project_id, &artifact.id)?;
    std::fs::create_dir_all(dir.join("screenshots"))?;
    let manifest = load_manifest(&dir, &artifact.id)?;
    let capture_html = artifact_capture_html(&artifact.project_id, &artifact.id)?;
    Ok(BaselinePreparation {
        artifact_id: artifact.id.clone(),
        artifact_hash: actual,
        dir,
        manifest,
        capture_html,
        scenarios_hash: scenarios.hash,
        plan,
        _artifact_lifecycle_guard: artifact_lifecycle_guard,
    })
}

fn finish_baseline(
    mut prepared: BaselinePreparation,
    captures: Vec<(CaptureCase, Vec<u8>)>,
) -> Result<QualityManifest> {
    ensure_capture_set(&prepared.plan, &captures)?;
    if artifact_hash(&prepared.artifact_id)? != prepared.artifact_hash {
        anyhow::bail!("artifact changed during baseline capture");
    }
    ensure_scenarios_unchanged(
        &super::scenarios::get(&prepared.artifact_id)?,
        &prepared.scenarios_hash,
    )?;
    let now = chrono::Utc::now().to_rfc3339();
    for (case, bytes) in captures {
        let viewport = case.viewport;
        capture_image(&bytes, viewport)?;
        let image_hash = blake3::hash(&bytes).to_hex().to_string();
        write_atomic(
            &prepared
                .dir
                .join("screenshots")
                .join(format!("{image_hash}.png")),
            &bytes,
        )?;
        prepared.manifest.baselines.insert(
            capture_key(&case),
            BaselineEntry {
                scenario_id: case.scenario.id,
                viewport,
                image_hash,
                artifact_hash: prepared.artifact_hash.clone(),
                accepted_at: now.clone(),
            },
        );
    }
    let bytes = serde_json::to_vec_pretty(&prepared.manifest)?;
    write_atomic(&prepared.dir.join("manifest.json"), &bytes)?;
    Ok(prepared.manifest)
}

fn readiness_script(ready: &super::scenarios::ScenarioReady) -> Result<String> {
    let selector = serde_json::to_string(&ready.selector)?;
    let text = serde_json::to_string(ready.text.trim())?;
    Ok(format!(
        r#"(() => {{
        const matches = document.querySelectorAll({selector});
        if (matches.length !== 1) return false;
        const element = matches[0], rect = element.getBoundingClientRect(), style = getComputedStyle(element);
        return element.textContent.trim() === {text} && rect.width > 0 && rect.height > 0
          && rect.bottom > 0 && rect.right > 0 && rect.top < innerHeight && rect.left < innerWidth
          && style.display !== 'none' && style.visibility !== 'hidden' && Number(style.opacity) > 0;
    }})()"#
    ))
}

async fn capture_all(html: &str, plan: &[CaptureCase]) -> Result<Vec<(CaptureCase, Vec<u8>)>> {
    // CdpBackend is stateless over one global active target. The guard must
    // cover original-target capture, disposable-tab work, close, and restore.
    let _cdp_operation = ha_browser::browser::acquire_cdp_operation_guard().await;
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
    let tab = backend.new_page(None).await?;
    let _ = backend.select_page(&tab.target_id).await;
    let result = async {
        backend.prepare_capture_page().await?;
        let mut captures = Vec::with_capacity(plan.len());
        let mut total_bytes = 0usize;
        for case in plan {
        let viewport = case.viewport;
        backend.resize(viewport.width, viewport.height).await?;
        // Each case receives a fresh opaque iframe; state cannot bleed from
        // an earlier scenario or viewport. CSP and the managed-tab lock remain.
        let wrapper = capture_wrapper_script(html)?;
        let sandbox = backend.evaluate(&wrapper).await?;
        if sandbox.as_str() != Some("allow-scripts") {
            anyhow::bail!("quality capture sandbox was not installed");
        }
        let mut ready = false;
        for _ in 0..40 {
            ready = backend
                .evaluate("window.__dsQualityReady === true")
                .await?
                .as_bool()
                .unwrap_or(false);
            if ready {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        if !ready {
            anyhow::bail!("sandboxed artifact did not become ready for capture");
        }
        tokio::time::sleep(std::time::Duration::from_millis(350)).await;
            if let Some(assertion) = &case.scenario.ready {
                let message = serde_json::to_string(&serde_json::json!({
                    "type": "ds_scenario", "route": case.scenario.route,
                    "state": case.scenario.state
                }))?;
                backend.evaluate(&format!("document.querySelector('iframe').contentWindow.postMessage({message}, '*'); true")).await?;
                let check = readiness_script(assertion)?;
                let mut reached = false;
                for _ in 0..60 {
                    if backend.evaluate_sandboxed_frame(&check).await?.as_bool() == Some(true) {
                        reached = true;
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                anyhow::ensure!(reached, "visual_scenarios_incomplete: scenario {} did not reach its visible ready condition", case.scenario.id);
            }
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            if let Some(assertion) = &case.scenario.ready {
                anyhow::ensure!(backend.evaluate_sandboxed_frame(&readiness_script(assertion)?).await?.as_bool() == Some(true),
                    "visual_scenarios_incomplete: scenario state changed before capture");
            }
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
            total_bytes += bytes.len();
            anyhow::ensure!(total_bytes <= MAX_TOTAL_PNG_BYTES, "visual capture set exceeds 128 MiB");
            captures.push((case.clone(), bytes));
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
    let artifact_id = artifact_id.to_string();
    let prepared =
        ha_core::blocking::run_blocking(move || prepare_quality_run(&artifact_id)).await?;
    let captures = capture_all(&prepared.capture_html, &prepared.plan).await?;
    ha_core::blocking::run_blocking(move || finish_quality_run(prepared, captures)).await
}

/// 显式 owner 操作：以期望源码哈希防止把陈旧画面接受为当前基线。
pub async fn accept(input: AcceptBaselineInput) -> Result<QualityManifest> {
    let prepared = ha_core::blocking::run_blocking(move || prepare_baseline(input)).await?;
    let captures = capture_all(&prepared.capture_html, &prepared.plan).await?;
    ha_core::blocking::run_blocking(move || finish_baseline(prepared, captures)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_states() -> super::super::scenarios::ScenariosManifest {
        use super::super::scenarios::{ScenarioReady, ScenariosManifest};
        let mut manifest = ScenariosManifest::default();
        let mut empty = manifest.scenarios.remove(0);
        empty.id = "empty".into();
        empty.title = "Empty cart".into();
        empty.state = serde_json::json!({"status":"empty"});
        empty.ready = Some(ScenarioReady {
            selector: "#status".into(),
            text: "empty".into(),
        });
        let mut error = empty.clone();
        error.id = "error".into();
        error.title = "Error cart".into();
        error.state = serde_json::json!({"status":"error"});
        error.ready.as_mut().unwrap().text = "error".into();
        manifest.scenarios = vec![empty, error];
        manifest
    }

    #[test]
    fn two_states_have_six_independent_capture_and_baseline_identities() {
        let mut manifest = two_states();
        let plan = capture_plan(&manifest).unwrap();
        assert_eq!(plan.len(), 6);
        let keys: std::collections::BTreeSet<_> = plan.iter().map(capture_key).collect();
        assert_eq!(keys.len(), 6);
        let captures: Vec<_> = plan.iter().cloned().map(|c| (c, Vec::new())).collect();
        assert!(ensure_capture_set(&plan, &captures).is_ok());
        assert!(ensure_capture_set(&plan, &captures[..5]).is_err());
        let mut duplicate = captures.clone();
        duplicate[3] = duplicate[0].clone();
        assert!(ensure_capture_set(&plan, &duplicate).is_err());
        manifest.scenarios[0].state = serde_json::json!({"status":"updated"});
        let updated = capture_plan(&manifest).unwrap();
        assert_ne!(capture_key(&plan[0]), capture_key(&updated[0]));
        assert_eq!(capture_key(&plan[3]), capture_key(&updated[3]));
    }

    #[test]
    fn baseline_acceptance_binds_the_reviewed_scenario_version() {
        use super::super::scenarios::ScenariosEnvelope;
        let custom = ScenariosEnvelope {
            manifest: two_states(),
            hash: "reviewed".into(),
        };
        assert!(ensure_acceptance_scenarios(&custom, None).is_err());
        assert!(ensure_acceptance_scenarios(&custom, Some("stale")).is_err());
        assert!(ensure_acceptance_scenarios(&custom, Some("reviewed")).is_ok());
        let legacy = ScenariosEnvelope {
            manifest: Default::default(),
            hash: "legacy".into(),
        };
        assert!(ensure_acceptance_scenarios(&legacy, None).is_ok());
        let plan = capture_plan(&legacy.manifest).unwrap();
        assert_eq!(capture_key(&plan[0]), "1440x900");
        let entry: BaselineEntry = serde_json::from_value(serde_json::json!({
            "viewport":{"width":1440,"height":900}, "imageHash":"old", "artifactHash":"old", "acceptedAt":"old"
        })).unwrap();
        assert_eq!(entry.scenario_id, "default");
    }

    #[test]
    fn ready_conditions_and_custom_viewports_are_bounded() {
        let mut manifest = two_states();
        manifest.scenarios[0].ready.as_mut().unwrap().selector = "[".into();
        assert!(capture_plan(&manifest).is_err());
        manifest = two_states();
        manifest.scenarios[0].ready.as_mut().unwrap().text = " ".into();
        assert!(capture_plan(&manifest).is_err());
        manifest = two_states();
        manifest.scenarios[0].route = "/checkout".into();
        assert!(capture_plan(&manifest).is_err());
    }

    #[tokio::test]
    #[ignore = "explicit isolated Chrome smoke test; requires HA_DATA_DIR fixture marker"]
    async fn isolated_browser_captures_states_and_rejects_unreached_state() {
        let root = std::path::PathBuf::from(
            std::env::var_os("HA_DATA_DIR").expect("isolated HA_DATA_DIR"),
        );
        assert!(
            root.join(".design-capture-test").is_file(),
            "refuse a user data directory"
        );
        let chrome =
            std::env::var("HA_DESIGN_TEST_CHROME").expect("explicit installed Chrome executable");
        assert!(std::path::Path::new(&chrome).is_file());
        let mut config = serde_json::to_value(ha_core::config::AppConfig::default()).unwrap();
        config["browser"] = serde_json::json!({"profiles":{"managed":{
            "executablePath":chrome,"userDataDir":root.join("browser-profile"),
            "headless":true,"extraArgs":["--disable-background-networking","--disable-component-update"]
        }}});
        write_atomic(
            &root.join("config.json"),
            &serde_json::to_vec_pretty(&config).unwrap(),
        )
        .unwrap();
        assert!(
            ha_browser::browser::profile::resolve_profile("managed")
                .unwrap()
                .user_data_dir
                .starts_with(&root),
            "refuse a non-isolated browser profile"
        );
        let manifest = two_states();
        let plan = capture_plan(&manifest).unwrap();
        let html = format!(
            r#"<!doctype html><html><head><style>body{{margin:0}}main{{height:100vh;background:tomato;color:black;font:40px sans-serif}}</style></head><body><main id="status">initial</main><script>
            window.addEventListener('ds_scenario', e => {{ const s=e.detail.state.status; document.querySelector('#status').textContent=s; document.querySelector('main').style.background=s==='empty'?'lightblue':'tomato'; }});
            // Attempt to spoof a ready check in the authored realm: isolated inspection must ignore it.
            document.querySelectorAll = () => [{{textContent:'missing',getBoundingClientRect:()=>({{width:1,height:1,bottom:1,right:1,top:0,left:0}})}}];
        </script>{}</body></html>"#,
            super::super::renderer::ZOOM_FORWARD_SCRIPT
        );
        let backend = CdpBackend::new();
        let original = backend.new_page(None).await.unwrap();
        backend.select_page(&original.target_id).await.unwrap();
        backend.prepare_capture_page().await.unwrap();
        backend
            .evaluate(
                &capture_wrapper_script("<html><body><p id='status'>missing</p></body></html>")
                    .unwrap(),
            )
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(350)).await;
        backend.resize(800, 600).await.unwrap();
        let original_tabs: std::collections::BTreeSet<_> = backend
            .list_pages()
            .await
            .unwrap()
            .into_iter()
            .map(|tab| tab.target_id)
            .collect();
        let captures = capture_all(&html, &plan).await.unwrap();
        ensure_capture_set(&plan, &captures).unwrap();
        for (case, bytes) in &captures {
            capture_image(bytes, case.viewport).unwrap();
        }
        for i in 0..3 {
            let a = capture_image(&captures[i].1, captures[i].0.viewport).unwrap();
            let b = capture_image(&captures[i + 3].1, captures[i + 3].0.viewport).unwrap();
            assert!(
                compare_images(&a, &b).0 > 0.5,
                "states must actually render differently"
            );
        }
        let out = root.join("capture-evidence");
        std::fs::create_dir_all(&out).unwrap();
        for (case, png) in &captures {
            std::fs::write(
                out.join(format!("{}-{}.png", case.scenario.id, key(case.viewport))),
                png,
            )
            .unwrap();
        }
        let mut missing = plan[0].clone();
        missing.scenario.ready.as_mut().unwrap().text = "missing".into();
        assert!(capture_all(&html, &[missing])
            .await
            .unwrap_err()
            .to_string()
            .contains("did not reach"));
        // Exercise the real local artifact/manifest/baseline pipeline as well,
        // using only the explicit isolated data root and supplied HTML/JS.
        let project = super::super::service::create_project(
            serde_json::from_value(serde_json::json!({"title":"Scenario capture smoke"})).unwrap(),
        )
        .unwrap();
        let artifact = super::super::service::create_artifact(serde_json::from_value(
            serde_json::json!({"projectId":project.id,"title":"State fixture","kind":"web",
                "bodyHtml":"<main id='status'>initial</main>",
                "css":"html,body{margin:0}main{min-height:100vh;font:40px sans-serif;background:lightblue}",
                "js":"window.addEventListener('ds_scenario',e=>{document.querySelector('#status').textContent=e.detail.state.status;document.querySelector('main').style.background=e.detail.state.status==='empty'?'lightblue':'tomato'})"})
        ).unwrap()).unwrap();
        let initial = super::super::scenarios::get(&artifact.id).unwrap();
        let saved =
            super::super::scenarios::save(&artifact.id, &initial.hash, manifest.clone()).unwrap();
        let before = run(&artifact.id).await.unwrap();
        assert_eq!(before.diffs.len(), 6);
        assert!(before.diffs.iter().all(|diff| diff.baseline_hash.is_none()));
        let accepted = accept(AcceptBaselineInput {
            artifact_id: artifact.id.clone(),
            expected_artifact_hash: before.artifact_hash.clone(),
            expected_scenarios_hash: Some(before.scenarios_hash.clone()),
        })
        .await
        .unwrap();
        assert_eq!(accepted.baselines.len(), 6);
        let repeated = run(&artifact.id).await.unwrap();
        assert!(
            repeated.deterministic_passed,
            "same states must match their own baselines: {repeated:?}"
        );
        let mut changed = manifest;
        changed.scenarios[0].state = serde_json::json!({"status":"updated"});
        changed.scenarios[0].ready.as_mut().unwrap().text = "updated".into();
        super::super::scenarios::save(&artifact.id, &saved.hash, changed).unwrap();
        assert!(accept(AcceptBaselineInput {
            artifact_id: artifact.id.clone(),
            expected_artifact_hash: before.artifact_hash,
            expected_scenarios_hash: Some(before.scenarios_hash)
        })
        .await
        .unwrap_err()
        .to_string()
        .contains("reviewed scenarios changed"));
        let updated = run(&artifact.id).await.unwrap();
        assert!(updated
            .diffs
            .iter()
            .filter(|d| d.scenario_id == "empty")
            .all(|d| d.baseline_hash.is_none()));
        assert!(updated
            .diffs
            .iter()
            .filter(|d| d.scenario_id == "error")
            .all(|d| d.passed));
        std::fs::write(
            out.join("quality-run.json"),
            serde_json::to_vec_pretty(&repeated).unwrap(),
        )
        .unwrap();
        let status = backend.status().await.unwrap();
        assert_eq!(
            status.active_target_id.as_deref(),
            Some(original.target_id.as_str())
        );
        let remaining_tabs: std::collections::BTreeSet<_> =
            status.tabs.into_iter().map(|tab| tab.target_id).collect();
        assert_eq!(
            remaining_tabs, original_tabs,
            "capture must preserve exactly the pre-existing tabs on success and failure"
        );
        assert_eq!(
            backend
                .take_snapshot(SnapshotFormat::Role)
                .await
                .unwrap()
                .viewport,
            (800, 600)
        );
        backend.close_page(&original.target_id).await.unwrap();
        ha_browser::browser_ui::disconnect().await.unwrap();
    }

    #[test]
    fn custom_scenarios_never_reuse_default_page_evidence() {
        use super::super::scenarios::ScenariosManifest;
        let default = ScenariosManifest::default();
        assert!(capture_plan(&default).is_ok());
        let mut state = default.clone();
        state.scenarios[0].state = serde_json::json!({"checkout": "empty"});
        assert!(capture_plan(&state).is_err());
        state.scenarios[0].state = serde_json::json!({"checkout": "error"});
        assert!(capture_plan(&state).is_err());
        let mut route = default.clone();
        route.scenarios[0].route = "/checkout".into();
        assert!(capture_plan(&route).is_err());
        let mut multiple = default.clone();
        let mut other = multiple.scenarios[0].clone();
        other.id = "empty".into();
        multiple.scenarios.push(other);
        assert!(capture_plan(&multiple).is_err());
        let mut viewport = default.clone();
        viewport.viewports.insert(
            "phone".into(),
            super::super::scenarios::ScenarioViewport {
                width: 390,
                height: 844,
                label: None,
            },
        );
        viewport.scenarios[0].viewport_ids = vec!["phone".into()];
        assert_eq!(capture_plan(&viewport).unwrap().len(), 1);
        let mut renamed = default.clone();
        renamed.scenarios[0].id = "error".into();
        assert!(capture_plan(&renamed).is_err());
    }

    #[test]
    fn scenario_changes_invalidate_pending_captures() {
        use super::super::scenarios::{ScenariosEnvelope, ScenariosManifest};
        let manifest = ScenariosManifest::default();
        let before = ScenariosEnvelope {
            manifest: manifest.clone(),
            hash: "before".into(),
        };
        assert!(ensure_scenarios_unchanged(&before, "before").is_ok());
        assert!(ensure_scenarios_unchanged(&before, "after").is_err());
        let mut changed = before;
        changed.manifest.scenarios[0].state = serde_json::json!({"checkout":"error"});
        assert!(ensure_scenarios_unchanged(&changed, "before").is_err());
    }

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

    #[test]
    fn capture_wrapper_sandboxes_scripts_and_blocks_network() {
        let artifact = "<!-- <head> --><html data-spoof='<head>'><head data-value='>'><script>window.marker = '</script-safe>';</script></head><body></body></html>";
        let secured = inject_capture_csp(artifact).unwrap();
        let policy_pos = secured.find("Content-Security-Policy").unwrap();
        let script_pos = secured.find("<script>").unwrap();
        assert!(policy_pos < script_pos);
        assert!(secured.contains("connect-src 'none'"));
        assert!(secured.contains("default-src 'none'"));
        assert!(!secured.contains("<!-- <head> -->"));
        let parsed = scraper::Html::parse_document(&secured);
        let head = parsed
            .root_element()
            .child_elements()
            .find(|element| element.value().name() == "head")
            .unwrap();
        let first = head.child_elements().next().unwrap();
        assert_eq!(first.value().name(), "meta");
        assert_eq!(
            first.value().attr("http-equiv"),
            Some("Content-Security-Policy")
        );

        let wrapper = capture_wrapper_script(artifact).unwrap();
        assert!(wrapper.contains("setAttribute('sandbox', 'allow-scripts')"));
        assert!(wrapper.contains("frame.srcdoc"));
        assert!(!wrapper.contains("allow-same-origin"));
        assert!(!wrapper.contains("window.marker"));
    }
}
