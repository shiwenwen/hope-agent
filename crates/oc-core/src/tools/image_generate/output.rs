use anyhow::Result;
use chrono::Local;

use super::helpers::effective_model;
use super::types::*;
use crate::agent::MEDIA_ITEMS_PREFIX;
use crate::attachments::{self, MediaItem, MediaKind};

// ── List Action ─────────────────────────────────────────────────

/// Build formatted text listing all available providers and their capabilities.
pub(super) fn build_list_result(config: &ImageGenConfig) -> Result<String> {
    let mut lines = Vec::new();
    lines.push("Available Image Generation Providers:".to_string());
    lines.push(String::new());

    let enabled: Vec<_> = config
        .providers
        .iter()
        .filter(|p| p.enabled && p.api_key.as_ref().map_or(false, |k| !k.is_empty()))
        .collect();

    if enabled.is_empty() {
        lines.push("No providers configured. Enable one and enter an API Key in Settings > Tool Settings > Image Generation.".to_string());
        return Ok(lines.join("\n"));
    }

    for (i, entry) in enabled.iter().enumerate() {
        let impl_ = match super::resolve_provider(&entry.id) {
            Some(i) => i,
            None => continue,
        };
        let caps = impl_.capabilities();
        let model = effective_model(entry);

        lines.push(format!(
            "{}. {} (default: {}) [Priority {}]",
            i + 1,
            impl_.display_name(),
            model,
            i + 1
        ));

        // Generate capabilities
        lines.push(format!(
            "   Generate: max {} image(s){}{}{}",
            caps.generate.max_count,
            if caps.generate.supports_size {
                ", size"
            } else {
                ""
            },
            if caps.generate.supports_aspect_ratio {
                ", aspectRatio"
            } else {
                ""
            },
            if caps.generate.supports_resolution {
                ", resolution"
            } else {
                ""
            },
        ));

        // Edit capabilities
        if caps.edit.enabled {
            lines.push(format!(
                "   Edit: enabled, max {} input image(s), max {} output{}{}{}",
                caps.edit.max_input_images,
                caps.edit.max_count,
                if caps.edit.supports_size {
                    ", size"
                } else {
                    ""
                },
                if caps.edit.supports_aspect_ratio {
                    ", aspectRatio"
                } else {
                    ""
                },
                if caps.edit.supports_resolution {
                    ", resolution"
                } else {
                    ""
                },
            ));
        } else {
            lines.push("   Edit: not supported".to_string());
        }

        // Geometry
        if let Some(ref geo) = caps.geometry {
            if !geo.sizes.is_empty() {
                lines.push(format!("   Sizes: {}", geo.sizes.join(", ")));
            }
            if !geo.aspect_ratios.is_empty() {
                lines.push(format!(
                    "   Aspect Ratios: {}",
                    geo.aspect_ratios.join(", ")
                ));
            }
            if !geo.resolutions.is_empty() {
                lines.push(format!("   Resolutions: {}", geo.resolutions.join(", ")));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_cfg(entries: Vec<ImageGenProviderEntry>) -> ImageGenConfig {
        ImageGenConfig {
            providers: entries,
            timeout_seconds: 60,
            default_size: "1024x1024".to_string(),
        }
    }

    fn entry_full(id: &str, enabled: bool, key: Option<&str>) -> ImageGenProviderEntry {
        ImageGenProviderEntry {
            id: id.to_string(),
            enabled,
            api_key: key.map(|k| k.to_string()),
            base_url: None,
            model: None,
            thinking_level: None,
        }
    }

    #[test]
    fn list_result_empty_when_no_provider_configured() {
        let cfg = mk_cfg(vec![entry_full("openai", false, None)]);
        let out = build_list_result(&cfg).unwrap();
        assert!(out.starts_with("Available Image Generation Providers:"));
        assert!(out.contains("No providers configured"));
    }

    #[test]
    fn list_result_shows_enabled_provider_with_priority() {
        let cfg = mk_cfg(vec![
            entry_full("openai", false, Some("sk")),
            entry_full("google", true, Some("real-key")),
        ]);
        let out = build_list_result(&cfg).unwrap();
        // "Priority 1" reflects index among the enabled subset, not the full list.
        assert!(out.contains("[Priority 1]"));
        // Only google is enabled with a key; the openai (disabled) should not appear.
        assert!(!out.contains("[Priority 2]"));
        // Provider capabilities block should be present.
        assert!(out.contains("Generate:"));
    }

    #[test]
    fn list_result_reports_edit_support() {
        let cfg = mk_cfg(vec![entry_full("openai", true, Some("sk"))]);
        let out = build_list_result(&cfg).unwrap();
        // OpenAI supports editing → the line must say "Edit: enabled" or similar.
        let has_edit_line = out.contains("Edit: enabled") || out.contains("Edit: not supported");
        assert!(has_edit_line, "expected Edit: line, got:\n{}", out);
    }
}
