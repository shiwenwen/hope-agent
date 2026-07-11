//! `image` 形态：接线现有 `image_generate` Provider 栈，生成图片并内嵌进
//! **自包含产物**（data-uri，守「轻量自包含 HTML」红线）。
//!
//! 不复用 `tool_image_generate`（它解析 JSON args、做 failover、落 attachments 目录、
//! 返回带 `__MEDIA_ITEMS__` 头的字符串）——而是直接组合公共 provider trait：
//! `resolve_image_gen_config` + `resolve_provider` + `ImageGenParams` +
//! `ImageGenProviderImpl::generate`（全 `crate::tools::image_generate::*` 公共）。

use anyhow::{anyhow, Result};
use base64::Engine;

use super::renderer::{html_escape, ArtifactParts};
use crate::tools::image_generate::InputImage;
use crate::tools::image_generate::{
    effective_model, resolve_image_gen_config, resolve_provider, ImageGenParams, ImageGenResult,
};

/// 生图可选项（B0-4）：比例提示 + 参考图（图生图/编辑）。默认空 = 纯文生图（改动前行为）。
#[derive(Default)]
pub struct ImageGenOptions {
    /// 比例提示，如 "1:1" / "16:9" / "9:16"。
    pub aspect_ratio: Option<String>,
    /// 参考/输入图（图生图或编辑）。空 = 纯文生图。
    pub input_images: Vec<InputImage>,
    /// inpaint 蒙版（PNG，透明/涂画区=重绘区）。与恰一张 input_image 搭配走 OpenAI `/images/edits`。
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

/// 生成一张图片，返回原始字节 + mime。**按配置顺序在多个 provider 间 failover**——首选
/// 被限流 / 报错时自动尝试下一个可用 provider（对齐 `tool_image_generate` 的健壮性）。
async fn generate_image_bytes(prompt: &str, opts: &ImageGenOptions) -> Result<(Vec<u8>, String)> {
    if prompt.trim().is_empty() {
        anyhow::bail!("image prompt is empty");
    }
    let app_cfg = crate::config::cached_config();
    let cfg = resolve_image_gen_config(&app_cfg.image_generate).ok_or_else(|| {
        anyhow!("no image-generation provider configured (Settings → Tools → Image)")
    })?;
    let candidates: Vec<_> = cfg
        .providers
        .iter()
        .filter(|p| p.enabled && p.api_key.as_deref().is_some_and(|k| !k.is_empty()))
        .collect();
    if candidates.is_empty() {
        anyhow::bail!("no image-generation provider configured");
    }

    let mut last_err: Option<anyhow::Error> = None;
    for entry in candidates {
        let Some(provider) = resolve_provider(&entry.id) else {
            last_err = Some(anyhow!("unknown image provider '{}'", entry.id));
            continue;
        };
        let model = effective_model(entry);
        let Some(api_key) = entry.api_key.as_deref() else {
            continue;
        };
        let params = ImageGenParams {
            api_key,
            base_url: entry.base_url.as_deref(),
            model: &model,
            prompt,
            size: &cfg.default_size,
            n: 1,
            timeout_secs: cfg.timeout_seconds,
            extra: entry,
            aspect_ratio: opts.aspect_ratio.as_deref(),
            resolution: None,
            input_images: &opts.input_images,
            mask: opts.mask.as_deref(),
        };
        let started = std::time::Instant::now();
        match provider.generate(params).await {
            Ok(ImageGenResult { images, .. }) => {
                // Image providers return no token usage — record call + duration
                // only (KIND_IMAGE_GENERATION), matching `tool_image_generate`.
                record_image_usage(
                    &entry.id,
                    provider.display_name(),
                    &model,
                    started.elapsed().as_millis() as u64,
                    true,
                    None,
                    Some(images.len()),
                );
                if let Some(img) = images.into_iter().next() {
                    crate::app_info!(
                        "design",
                        "image",
                        "generated image {} bytes mime={} via provider={}",
                        img.data.len(),
                        img.mime,
                        entry.id
                    );
                    return Ok((img.data, img.mime));
                }
                last_err = Some(anyhow!("image provider '{}' returned no images", entry.id));
            }
            Err(e) => {
                record_image_usage(
                    &entry.id,
                    provider.display_name(),
                    &model,
                    started.elapsed().as_millis() as u64,
                    false,
                    Some(e.to_string()),
                    None,
                );
                crate::app_warn!(
                    "design",
                    "image",
                    "image provider '{}' failed, trying next: {e}",
                    entry.id
                );
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("all image providers failed")))
}

/// Record one design image-generation attempt (`KIND_IMAGE_GENERATION`, owner
/// plane so no session/agent id). Images carry no token usage — call count +
/// duration only, per the usage-ledger contract.
fn record_image_usage(
    provider_id: &str,
    provider_name: &str,
    model: &str,
    duration_ms: u64,
    success: bool,
    error: Option<String>,
    output_image_count: Option<usize>,
) {
    let mut event =
        crate::model_usage::ModelUsageEvent::new(crate::model_usage::KIND_IMAGE_GENERATION);
    event.operation = Some("design.image".to_string());
    event.source = Some("design.image".to_string());
    event.provider_id = Some(provider_id.to_string());
    // Human display name (matches `tool_image_generate`) so the Dashboard
    // "by model" GROUP BY (model_id, provider_name) doesn't fragment the same
    // provider/model into two rows across the two image entry points.
    event.provider_name = Some(provider_name.to_string());
    event.model_id = Some(model.to_string());
    event.duration_ms = Some(duration_ms);
    event.success = success;
    event.error = error;
    event.metadata = Some(serde_json::json!({ "output_image_count": output_image_count }));
    crate::model_usage::record_model_usage_best_effort(event);
}
