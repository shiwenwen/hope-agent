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
            CaptureKind::Pdf => backend
                .save_pdf(PdfParams {
                    print_background: Some(true),
                    ..Default::default()
                })
                .await
                .context("printToPDF failed"),
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
pub async fn capture_artifact_b64(artifact_id: &str, format: &str) -> Result<(String, String)> {
    let kind = CaptureKind::parse(format)
        .with_context(|| format!("unsupported native export format: {format}"))?;
    let bytes = capture_artifact(artifact_id, kind).await?;
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok((b64, kind.mime().to_string()))
}
