//! `image` 形态：接线统一媒体生成栈（`crate::media_gen`），生成图片并内嵌进
//! **自包含产物**（data-uri，守「轻量自包含 HTML」红线）。
//!
//! 不复用 `tool_image_generate`（它解析 JSON args、落 attachments 目录、返回带
//! `__MEDIA_ITEMS__` 头的字符串）——但 failover / 能力校验 / 记账与它共用同一
//! `media_gen::execute_image` 执行器（消灭旧版三份重复的 provider 循环）。

use anyhow::{anyhow, Result};
use base64::Engine;

use super::renderer::{html_escape, ArtifactParts};
use crate::media_gen::adapters::InputImage;
use crate::media_gen::{execute_image, ImageRequest, UsageMeta};

/// 生图可选项（B0-4 + 参数透传）：几何参数 + 参考图（图生图/编辑）。默认空 =
/// 纯文生图、几何全落 `media_gen.image_defaults`。
#[derive(Default)]
pub struct ImageGenOptions {
    /// 比例提示，如 "1:1" / "16:9" / "9:16"。
    pub aspect_ratio: Option<String>,
    /// 尺寸，如 "1024x1024"；None = 全局默认。
    pub size: Option<String>,
    /// 分辨率档："1K" / "2K" / "4K"；None = 全局默认。
    pub resolution: Option<String>,
    /// 参考/输入图（图生图或编辑）。空 = 纯文生图。
    pub input_images: Vec<InputImage>,
    /// inpaint 蒙版（PNG，透明/涂画区=重绘区）。只投给 `supports_mask` 的模型。
    pub mask: Option<Vec<u8>>,
}

/// 把图片字节内嵌成 `image` 形态 body（一张居中图，data-uri，守自包含红线）。
/// 拖入导入 / 生成两条路径共用同一 body 结构。
pub fn image_body_from_bytes(bytes: &[u8], mime: &str, alt: &str) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    let alt = html_escape(alt);
    let mime = if mime.trim().is_empty() {
        "image/png"
    } else {
        mime
    };
    format!(
        "<img src=\"data:{mime};base64,{b64}\" alt=\"{alt}\" \
style=\"display:block;margin:0 auto;max-width:100%;height:auto\">"
    )
}

/// 文本 prompt → 生成图片 → 返回内嵌 data-uri 的 `ArtifactParts`（body 一张居中图）。
pub async fn generate_image_parts(
    prompt: &str,
    alt: &str,
    opts: &ImageGenOptions,
) -> Result<ArtifactParts> {
    let (bytes, mime) = generate_image_bytes(prompt, opts).await?;
    Ok(ArtifactParts {
        body_html: image_body_from_bytes(&bytes, &mime, alt),
        css: String::new(),
        js: String::new(),
    })
}

/// 生成一张图片，返回原始字节 + mime。链解析 / failover / 重试 / 记账全在
/// `media_gen::execute_image`（KIND_IMAGE_GENERATION，owner 平面无 session id）。
async fn generate_image_bytes(prompt: &str, opts: &ImageGenOptions) -> Result<(Vec<u8>, String)> {
    if prompt.trim().is_empty() {
        anyhow::bail!("image prompt is empty");
    }
    let app_cfg = crate::config::cached_config();
    let outcome = execute_image(
        &app_cfg.media_gen,
        ImageRequest {
            prompt,
            size: opts.size.as_deref(),
            n: 1,
            aspect_ratio: opts.aspect_ratio.as_deref(),
            resolution: opts.resolution.as_deref(),
            input_images: &opts.input_images,
            mask: opts.mask.as_deref(),
            explicit_model: None,
        },
        UsageMeta {
            operation: "design.image",
            source: "design.image",
            session_id: None,
            agent_id: None,
        },
    )
    .await?;

    let provider_name = outcome.provider_name;
    let img = outcome
        .result
        .images
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("image provider '{provider_name}' returned no images"))?;
    crate::app_info!(
        "design",
        "image",
        "generated image {} bytes mime={} via {}/{}",
        img.data.len(),
        img.mime,
        provider_name,
        outcome.model_id
    );
    Ok((img.data, img.mime))
}
