use anyhow::Result;
use chrono::Local;

use crate::agent::MEDIA_ITEMS_PREFIX;
use crate::attachments::{self, MediaItem, MediaKind};
use crate::media_gen::adapters::ImageGenResult;
use crate::media_gen::{MediaGenConfig, MediaModality};

// ── List Action ─────────────────────────────────────────────────

/// Build formatted text listing all usable providers and their image
/// models with data-driven capabilities.
pub(super) fn build_list_result(config: &MediaGenConfig) -> Result<String> {
    let mut lines = Vec::new();
    lines.push("Available Image Generation Providers:".to_string());
    lines.push(String::new());

    let usable: Vec<_> = config
        .providers
        .iter()
        .filter(|p| p.is_usable() && p.models.iter().any(|m| m.modality == MediaModality::Image))
        .collect();

    if usable.is_empty() {
        lines.push(
            "No providers configured. Add one in Settings → Model Providers → Generation Models."
                .to_string(),
        );
        return Ok(lines.join("\n"));
    }

    if let Some(chain) = &config.chains.image {
        let refs: Vec<String> = chain.iter().map(|r| r.model_id.clone()).collect();
        lines.push(format!("Default chain: {}", refs.join(" → ")));
        lines.push(String::new());
    }

    for (i, provider) in usable.iter().enumerate() {
        lines.push(format!("{}. {} [Priority {}]", i + 1, provider.name, i + 1));
        for model in provider
            .models
            .iter()
            .filter(|m| m.modality == MediaModality::Image)
        {
            let Some(caps) = &model.image else {
                lines.push(format!("   - {} (capabilities unknown)", model.id));
                continue;
            };
            let mut feats: Vec<String> = vec![format!("max {} image(s)", caps.max_n)];
            if caps.supports_size {
                feats.push("size".into());
            }
            if caps.supports_aspect_ratio {
                feats.push("aspectRatio".into());
            }
            if caps.supports_resolution {
                feats.push("resolution".into());
            }
            if caps.supports_mask {
                feats.push("mask inpaint".into());
            }
            if let Some(edit) = &caps.edit {
                feats.push(format!("edit (≤{} input)", edit.max_input_images));
            }
            lines.push(format!("   - {}: {}", model.id, feats.join(", ")));
            if !caps.sizes.is_empty() {
                lines.push(format!("     Sizes: {}", caps.sizes.join(", ")));
            }
            if !caps.aspect_ratios.is_empty() {
                lines.push(format!(
                    "     Aspect Ratios: {}",
                    caps.aspect_ratios.join(", ")
                ));
            }
            if !caps.resolutions.is_empty() {
                lines.push(format!("     Resolutions: {}", caps.resolutions.join(", ")));
            }
        }
        lines.push(String::new());
    }

    Ok(lines.join("\n"))
}

// ── Success Result Builder ──────────────────────────────────────

/// Build the success result string with failover transparency.
///
/// Writes generated images into the session's attachments directory (or the
/// shared `_temp` bucket when no session id is available yet) so the HTTP
/// `/api/attachments/{sid}/{filename}` endpoint can serve them. Emits the
/// `__MEDIA_ITEMS__` structured header so the event system produces a
/// unified `media_items[]` payload shared with `send_attachment`.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_success_result(
    gen_result: ImageGenResult,
    display_name: &str,
    model: &str,
    size: &str,
    aspect_ratio: Option<&str>,
    resolution: Option<&str>,
    is_edit: bool,
    failover_log: &[String],
    session_id: Option<&str>,
) -> Result<String> {
    let images = gen_result.images;
    let accompanying_text = gen_result.text;

    let timestamp = Local::now().format("%Y%m%d_%H%M%S");
    let mut media_items: Vec<MediaItem> = Vec::with_capacity(images.len());

    for (i, img) in images.iter().enumerate() {
        let ext = if img.mime.contains("jpeg") || img.mime.contains("jpg") {
            "jpg"
        } else {
            "png"
        };
        let display_filename = format!("{}_{}.{}", timestamp, i, ext);
        let saved_path =
            match attachments::save_attachment_bytes(session_id, &display_filename, &img.data) {
                Ok(p) => p,
                Err(e) => {
                    app_warn!(
                        "tool",
                        "image_generate",
                        "Failed to save generated image: {}",
                        e
                    );
                    continue;
                }
            };
        media_items.push(MediaItem::from_saved_path(
            session_id,
            &saved_path,
            &display_filename,
            img.mime.clone(),
            img.data.len() as u64,
            MediaKind::Image,
            img.revised_prompt.clone(),
        ));
    }

    // Build result string
    let mut text_parts = Vec::new();
    let action_word = if is_edit { "Edited" } else { "Generated" };
    text_parts.push(format!(
        "{} {} image{} with {}/{}.",
        action_word,
        images.len(),
        if images.len() > 1 { "s" } else { "" },
        display_name,
        model
    ));
    text_parts.push(format!("Size: {}", size));

    if let Some(ar) = aspect_ratio {
        text_parts.push(format!("Aspect Ratio: {}", ar));
    }
    if let Some(res) = resolution {
        text_parts.push(format!("Resolution: {}", res));
    }

    // Report failover if it occurred
    if !failover_log.is_empty() {
        text_parts.push(format!("[Failover] {}", failover_log.join(" → ")));
    }

    let saved_paths: Vec<&str> = media_items
        .iter()
        .filter_map(|it| it.local_path.as_deref())
        .collect();
    for p in &saved_paths {
        text_parts.push(format!("Saved to: {}", p));
    }
    if !images.is_empty() {
        if let Some(ref rp) = images[0].revised_prompt {
            text_parts.push(format!("Revised prompt: {}", rp));
        }
    }
    if let Some(ref text) = accompanying_text {
        text_parts.push(format!("Model response: {}", text));
    }

    let items_json = serde_json::to_string(&media_items).unwrap_or_else(|_| "[]".to_string());
    let result = format!(
        "{}{}\n{}",
        MEDIA_ITEMS_PREFIX,
        items_json,
        text_parts.join("\n")
    );

    let revised_prompts: Vec<&str> = images
        .iter()
        .filter_map(|img| img.revised_prompt.as_deref())
        .collect();
    let image_sizes: Vec<usize> = images.iter().map(|img| img.data.len()).collect();
    let mime_types: Vec<&str> = images.iter().map(|img| img.mime.as_str()).collect();
    if let Some(logger) = crate::get_logger() {
        let text_preview = accompanying_text.as_deref().map(|t| {
            if t.len() > 500 {
                format!("{}...", crate::truncate_utf8(t, 500))
            } else {
                t.to_string()
            }
        });
        logger.log(
            "info",
            "tool",
            "image_generate",
            &format!(
                "Image generation complete: {} image(s), {} saved, provider={}/{}, edit={}",
                images.len(),
                saved_paths.len(),
                display_name,
                model,
                is_edit
            ),
            Some(
                serde_json::json!({
                    "provider": display_name,
                    "model": model,
                    "size": size,
                    "aspect_ratio": aspect_ratio,
                    "resolution": resolution,
                    "is_edit": is_edit,
                    "image_count": images.len(),
                    "image_sizes_bytes": image_sizes,
                    "mime_types": mime_types,
                    "saved_paths": &saved_paths,
                    "revised_prompts": revised_prompts,
                    "accompanying_text": text_preview,
                    "failover_log": failover_log,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }

    Ok(result)
}
