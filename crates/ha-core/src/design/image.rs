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
use crate::tools::image_generate::{
    effective_model, resolve_image_gen_config, resolve_provider, ImageGenParams, ImageGenResult,
};

/// 文本 prompt → 生成图片 → 返回内嵌 data-uri 的 `ArtifactParts`（body 一张居中图）。
pub async fn generate_image_parts(prompt: &str, alt: &str) -> Result<ArtifactParts> {
    let (bytes, mime) = generate_image_bytes(prompt).await?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let alt = html_escape(alt);
    let body_html = format!(
        "<img src=\"data:{mime};base64,{b64}\" alt=\"{alt}\" \
style=\"display:block;margin:0 auto;max-width:100%;height:auto\">"
    );
    Ok(ArtifactParts {
        body_html,
        css: String::new(),
        js: String::new(),
    })
}

/// 生成一张图片，返回原始字节 + mime。无可用 provider → Err。
async fn generate_image_bytes(prompt: &str) -> Result<(Vec<u8>, String)> {
    if prompt.trim().is_empty() {
        anyhow::bail!("image prompt is empty");
    }
    let app_cfg = crate::config::cached_config();
    let cfg = resolve_image_gen_config(&app_cfg.image_generate).ok_or_else(|| {
        anyhow!("no image-generation provider configured (Settings → Tools → Image)")
    })?;
    // 首个 enabled + 有 key 的 provider = 最高优先级（与 tool 同口径）。
    let entry = cfg
        .providers
        .iter()
        .find(|p| p.enabled && p.api_key.as_deref().is_some_and(|k| !k.is_empty()))
        .ok_or_else(|| anyhow!("no image-generation provider configured"))?;
    let provider = resolve_provider(&entry.id)
        .ok_or_else(|| anyhow!("unknown image provider '{}'", entry.id))?;
    let model = effective_model(entry);
    let api_key = entry
        .api_key
        .as_deref()
        .ok_or_else(|| anyhow!("image provider missing api key"))?;

    let params = ImageGenParams {
        api_key,
        base_url: entry.base_url.as_deref(),
        model: &model,
        prompt,
        size: &cfg.default_size,
        n: 1,
        timeout_secs: cfg.timeout_seconds,
        extra: entry,
        aspect_ratio: None,
        resolution: None,
        input_images: &[],
    };
    let ImageGenResult { images, .. } = provider.generate(params).await?;
    let img = images
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("image provider returned no images"))?;
    crate::app_info!(
        "design",
        "image",
        "generated image {} bytes mime={}",
        img.data.len(),
        img.mime
    );
    Ok((img.data, img.mime))
}
