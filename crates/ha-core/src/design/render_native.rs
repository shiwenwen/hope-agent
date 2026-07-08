//! 强路导出：用**真实浏览器**在隔离页里渲染产物并原生捕获——PDF 走 `printToPDF`
//! （**矢量、文字可选可搜**），PNG 走 `captureScreenshot`（**全保真**，彻底摆脱 html2canvas
//! 的 CSS 子集天花板）。
//!
//! 复用现有 CDP 浏览器后端（`crate::browser`）：Chromium **按需下载、不打进安装包**，
//! CDP + `save_pdf` + `take_screenshot` 都是浏览器工具已在用的成熟能力。后端不可用时上层
//! 回退客户端 html2canvas / jsPDF 路径（见 `src/lib/designExport.ts`）。
//!
//! **不驻留标签**：独立 `new_page` 打开、捕获后 `close_page` 收尾（无论成败）。

use anyhow::{Context, Result};

use crate::browser::{ImageFormat, PdfParams, ScreenshotParams};

/// 原生捕获格式。
#[derive(Clone, Copy, Debug)]
pub enum CaptureKind {
    /// 矢量 PDF（printToPDF）。
    Pdf,
    /// 全保真 PNG（captureScreenshot，整页）。
    Png,
}

impl CaptureKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pdf" => Some(Self::Pdf),
            "png" => Some(Self::Png),
            _ => None,
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Self::Pdf => "application/pdf",
            Self::Png => "image/png",
        }
    }
}

/// 导出强路的浏览器引擎依赖三态（PDF/PNG 强路预检用）。与 [`crate::ffmpeg::FfmpegStatus`]
/// 同 camelCase JSON 形，前端共用一个类型。`ready` = 有系统浏览器或已下载的 Chromium runtime。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserExportStatus {
    pub ready: bool,
    /// `system` | `runtime` | `missing`.
    pub source: String,
    pub binary_path: Option<String>,
    pub can_auto_install: bool,
}

/// 探测导出强路可用的浏览器引擎：系统 Chrome/Edge/Brave/Chromium → 已下载的 Chromium runtime
/// → 缺失。`can_auto_install` = 本平台有 Chromium 按需下载源。
pub fn browser_export_status() -> BrowserExportStatus {
    let can_auto_install = crate::browser::runtime::spec_for_current_platform().is_some();
    if let Some(p) = crate::platform::find_chrome_executable() {
        return BrowserExportStatus {
            ready: true,
            source: "system".into(),
            binary_path: Some(p.to_string_lossy().into_owned()),
            can_auto_install,
        };
    }
    if let Some(p) = crate::browser::runtime::cached_binary_path() {
        return BrowserExportStatus {
            ready: true,
            source: "runtime".into(),
            binary_path: Some(p.to_string_lossy().into_owned()),
            can_auto_install,
        };
    }
    BrowserExportStatus {
        ready: false,
        source: "missing".into(),
        binary_path: None,
        can_auto_install,
    }
}

/// 用真实浏览器渲染产物 `index.html` 并原生捕获为 PDF / PNG 字节。
///
/// 失败（无后端 / 渲染出错）返回 `Err`，由 owner 层决定回退客户端路径。
pub async fn capture_artifact(artifact_id: &str, kind: CaptureKind) -> Result<Vec<u8>> {
    let db = super::service::open_db()?;
    let a = db
        .get_artifact(artifact_id)?
        .with_context(|| format!("artifact not found: {artifact_id}"))?;
    let dir = crate::paths::design_artifact_dir(&a.project_id, &a.id)?;
    let index = dir.join("index.html");
    if !index.exists() {
        anyhow::bail!("artifact has no rendered index.html to capture");
    }
    // file:// URL——自包含产物的相对 CSS/JS/图片都在同目录，可直接加载。
    let url = format!("file://{}", index.to_string_lossy());

    let backend = crate::browser::acquire_backend()
        .await
        .context("no browser backend available for native export")?;

    // 隔离新页（不碰用户其它标签）；用完必关。
    let tab = backend
        .new_page(Some(&url))
        .await
        .context("failed to open export page")?;

    let capture = async {
        let _ = backend.select_page(&tab.target_id).await;
        // new_page 可能先落到空白页（Chrome 先开 new-tab），未真正到目标就补一次 navigate。
        if !tab.url.starts_with("file://") {
            backend
                .navigate(&url)
                .await
                .context("failed to navigate export page")?;
        }
        // 等字体 / 布局稳定后再捕获。
        crate::app_info!(
            "design",
            "render_native",
            "native capture {kind:?} for {artifact_id}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;

        match kind {
            CaptureKind::Pdf => {
                // Deck：横向 + 让页内 `@page{size:1280px 720px}`（renderer 的 @media print）
                // 决定纸张——一张幻灯片一页、满幅不裁（B7-3 修复：此前裸默认=Letter 竖版只印首张）。
                let is_deck = a.kind == "deck";
                backend
                    .save_pdf(PdfParams {
                        print_background: Some(true),
                        landscape: if is_deck { Some(true) } else { None },
                        prefer_css_page_size: if is_deck { Some(true) } else { None },
                        ..Default::default()
                    })
                    .await
                    .context("printToPDF failed")
            }
            CaptureKind::Png => backend
                .take_screenshot(ScreenshotParams {
                    format: ImageFormat::Png,
                    full_page: true,
                    ..Default::default()
                })
                .await
                .context("captureScreenshot failed"),
        }
    }
    .await;

    // 收尾：无论成败都关掉导出页，不留隔离标签。
    let _ = backend.close_page(&tab.target_id).await;
    capture
}

/// 捕获并 base64 编码，供 owner 命令（Tauri / HTTP）直接返回。返回 `(base64, mime)`。
/// `format` ∈ `pdf` / `png`（单帧原生捕获）/ `video`|`mp4`（逐帧真渲染 + ffmpeg 编码）。
pub async fn capture_artifact_b64(artifact_id: &str, format: &str) -> Result<(String, String)> {
    use base64::Engine;
    if format == "video" || format == "mp4" {
        let bytes = capture_video(artifact_id, 30, 120).await?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        return Ok((b64, "video/mp4".to_string()));
    }
    let kind = CaptureKind::parse(format)
        .with_context(|| format!("unsupported native export format: {format}"))?;
    let bytes = capture_artifact(artifact_id, kind).await?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok((b64, kind.mime().to_string()))
}

/// 确定性时钟 harness（与 `src/lib/designVideo.ts` 同源）：patch rAF / performance.now /
/// Date.now 为虚拟时钟 + WAAPI `getAnimations()` 定格；暴露 `__dsSeek(ms)` / `__dsDuration()`。
/// 注入进 `</head>` 前，保证在产物脚本前生效。
const VIDEO_HARNESS: &str = r#"<script>
(function(){
  var vt=0, cbs=[], nid=1;
  try{Object.defineProperty(performance,'now',{value:function(){return vt},configurable:true});}
  catch(e){try{performance.now=function(){return vt};}catch(_){}}
  try{Date.now=function(){return vt};}catch(e){}
  window.requestAnimationFrame=function(fn){var id=nid++;cbs.push([id,fn]);return id;};
  window.cancelAnimationFrame=function(id){cbs=cbs.filter(function(c){return c[0]!==id;});};
  window.__dsSeek=function(ms){
    vt=ms; var p=cbs; cbs=[];
    p.forEach(function(c){try{c[1](ms);}catch(e){}});
    try{(document.getAnimations?document.getAnimations():[]).forEach(function(a){
      try{a.pause();a.currentTime=ms;}catch(e){}});}catch(e){}
  };
  window.__dsDuration=function(){
    var s=document.querySelector('.ds-stage');
    var d=s&&s.getAttribute('data-ds-duration');
    if(d&&+d>0)return +d;
    var max=0;
    try{(document.getAnimations?document.getAnimations():[]).forEach(function(a){
      try{var ct=a.effect&&a.effect.getComputedTiming?a.effect.getComputedTiming():null;
      if(ct&&isFinite(ct.endTime))max=Math.max(max,ct.endTime);}catch(e){}});}catch(e){}
    return max;
  };
})();
</script>"#;

/// ffmpeg 二进制：`HA_FFMPEG_PATH` 覆盖 → 按需下载的缓存 runtime → PATH 上的 `ffmpeg`
/// （单一真相源 [`crate::ffmpeg::resolve_bin`]）。全缺失则编码整体 Err，前端回退客户端
/// WebCodecs——导出流程会先经 `ffmpeg::doctor` 预检、缺则引导下载/安装，不再静默降级。
fn ffmpeg_bin() -> String {
    crate::ffmpeg::resolve_bin()
}

/// 视频强路：真实浏览器逐帧渲染（确定性时钟定格）→ 每帧原生截图 → ffmpeg 编码 MP4。
/// 无浏览器后端 / 无 ffmpeg / 编码失败均返回 Err，由上层回退客户端 WebCodecs。
pub async fn capture_video(artifact_id: &str, fps: u32, max_secs: u32) -> Result<Vec<u8>> {
    let fps = fps.clamp(10, 60);
    let db = super::service::open_db()?;
    let a = db
        .get_artifact(artifact_id)?
        .with_context(|| format!("artifact not found: {artifact_id}"))?;
    let dir = crate::paths::design_artifact_dir(&a.project_id, &a.id)?;
    let index = dir.join("index.html");
    let html = std::fs::read_to_string(&index).context("artifact has no index.html to capture")?;
    // 注入 harness（在 </head> 前，保证先于产物脚本），落成产物目录内的临时 HTML——同目录
    // 相对 CSS/JS/图片才能被 file:// 正确加载。
    let injected = match html.find("</head>") {
        Some(i) => format!("{}{}{}", &html[..i], VIDEO_HARNESS, &html[i..]),
        None => format!("{VIDEO_HARNESS}{html}"),
    };
    let uniq = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_html = dir.join(format!(".ds-export-{uniq}.html"));
    let work = std::env::temp_dir().join(format!("ds-video-{uniq}"));
    std::fs::create_dir_all(&work)?;
    std::fs::write(&temp_html, injected.as_bytes())?;
    let url = format!("file://{}", temp_html.to_string_lossy());

    let run = capture_video_inner(&url, fps, max_secs, &work).await;

    // 收尾：删临时 HTML + 帧目录（无论成败）。
    let _ = std::fs::remove_file(&temp_html);
    let _ = std::fs::remove_dir_all(&work);
    run
}

async fn capture_video_inner(
    url: &str,
    fps: u32,
    max_secs: u32,
    work: &std::path::Path,
) -> Result<Vec<u8>> {
    let backend = crate::browser::acquire_backend()
        .await
        .context("no browser backend available for native video export")?;
    let tab = backend
        .new_page(Some(url))
        .await
        .context("failed to open export page")?;

    let frames = async {
        let _ = backend.select_page(&tab.target_id).await;
        if !tab.url.contains(".ds-export-") {
            backend
                .navigate(url)
                .await
                .context("navigate export page")?;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // 时长：__dsDuration()（data-ds-duration / WAAPI 最长），兜底 6s，钳 [1s, max]。
        let dur_val = backend
            .evaluate("__dsDuration()")
            .await
            .unwrap_or(serde_json::Value::Null);
        let mut dur_ms = dur_val.as_f64().filter(|d| *d > 0.0).unwrap_or(6000.0);
        let max_ms = (max_secs.clamp(1, 300) as f64) * 1000.0;
        dur_ms = dur_ms.clamp(1000.0, max_ms);
        let total = (((dur_ms / 1000.0) * fps as f64).round() as u32).max(1);
        crate::app_info!(
            "design",
            "render_native",
            "video capture {total} frames @ {fps}fps ({dur_ms}ms)"
        );

        for i in 0..total {
            let t = (i as f64 / fps as f64) * 1000.0;
            let _ = backend.evaluate(&format!("__dsSeek({t})")).await;
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
            let png = backend
                .take_screenshot(ScreenshotParams {
                    format: ImageFormat::Png,
                    full_page: false,
                    ..Default::default()
                })
                .await
                .context("frame screenshot failed")?;
            std::fs::write(work.join(format!("f_{i:05}.png")), &png)?;
        }
        Ok::<u32, anyhow::Error>(total)
    }
    .await;

    let _ = backend.close_page(&tab.target_id).await;
    let total = frames?;
    if total == 0 {
        anyhow::bail!("no frames captured");
    }

    // ffmpeg 编码（阻塞子进程放 spawn_blocking，不占 async 执行器）。
    let bin = ffmpeg_bin();
    let out = work.join("out.mp4");
    let frames_glob = work.join("f_%05d.png");
    let out_clone = out.clone();
    let status = tokio::task::spawn_blocking(move || {
        std::process::Command::new(bin)
            .arg("-y")
            .args(["-framerate", &fps.to_string()])
            .arg("-i")
            .arg(&frames_glob)
            .args([
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
            ])
            .arg(&out_clone)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
    })
    .await
    .context("ffmpeg task panicked")?
    .context(
        "ffmpeg not available — install ffmpeg or set HA_FFMPEG_PATH (falls back to client)",
    )?;
    if !status.success() {
        anyhow::bail!("ffmpeg encoding failed");
    }
    std::fs::read(&out).context("failed to read ffmpeg output")
}
