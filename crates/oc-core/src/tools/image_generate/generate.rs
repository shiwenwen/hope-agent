use std::collections::HashSet;

use anyhow::Result;
use serde_json::Value;

use crate::provider;
use super::helpers::*;
use super::output::*;
use super::types::*;

// ── Tool Entry Point (with Failover) ────────────────────────────

pub(crate) async fn tool_image_generate(args: &Value) -> Result<String> {
    let config = provider::load_store()
        .map(|s| s.image_generate)
        .unwrap_or_default();

    // Parse action
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("generate");

    if action == "list" {
        return build_list_result(&config);
    }

    if action != "generate" {
        anyhow::bail!("Invalid action '{}'. Must be 'generate' or 'list'.", action);
    }

    let prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'prompt' parameter"))?;

    let size = args
        .get("size")
        .and_then(|v| v.as_str())
        .unwrap_or(&config.default_size);

    let n = args.get("n").and_then(|v| v.as_u64()).unwrap_or(1).max(1) as u32;

    let model_override = args
        .get("model")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty() && *s != "auto");

    // Parse aspectRatio
    let aspect_ratio = args
        .get("aspectRatio")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    if let Some(ar) = aspect_ratio {
        if !VALID_ASPECT_RATIOS.contains(&ar) {
            anyhow::bail!(
                "Invalid aspectRatio '{}'. Must be one of: {}",
                ar,
                VALID_ASPECT_RATIOS.join(", ")
            );
        }
    }

    // Parse resolution
    let resolution = args
        .get("resolution")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    if let Some(res) = resolution {
        if !VALID_RESOLUTIONS.contains(&res) {
            anyhow::bail!(
                "Invalid resolution '{}'. Must be one of: {}",
                res,
                VALID_RESOLUTIONS.join(", ")
            );
        }
    }

    // Load input/reference images
    let mut image_paths: Vec<String> = Vec::new();
    if let Some(single) = args
        .get("image")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        image_paths.push(single.to_string());
    }
    if let Some(arr) = args.get("images").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(s) = item.as_str() {
                let trimmed = s.trim().to_string();
                if !trimmed.is_empty() {
                    image_paths.push(trimmed);
                }
            }
        }
    }
    // Deduplicate
    {
        let mut seen = HashSet::new();
        image_paths.retain(|p| {
            let key = p.trim_start_matches('@').trim().to_string();
            seen.insert(key)
        });
    }
    if image_paths.len() > MAX_INPUT_IMAGES {
        anyhow::bail!(
            "Too many reference images: {} provided, maximum is {}.",
            image_paths.len(),
            MAX_INPUT_IMAGES
        );
    }

    let mut input_images: Vec<InputImage> = Vec::new();
    for path in &image_paths {
        let clean = path.trim_start_matches('@').trim();
        match load_input_image(clean).await {
            Ok(img) => input_images.push(img),
            Err(e) => anyhow::bail!("Failed to load reference image '{}': {}", clean, e),
        }
    }

    let is_edit = !input_images.is_empty();

    // Auto-infer resolution from input images when editing
    let effective_resolution = if resolution.is_some() {
        resolution
    } else if is_edit && size == config.default_size {
        // Only auto-infer if no explicit size/resolution
        Some(infer_resolution(&input_images))
    } else {
        None
    };

    // Build candidate list
    let candidates: Vec<&ImageGenProviderEntry> = if let Some(model_name) = model_override {
        // Explicit model → find its provider (no failover)
        match find_provider_by_model(model_name, &config) {
            Some(entry) => vec![entry],
            None => anyhow::bail!(
                "Model '{}' not available. Configure it in Settings > Tool Settings > Image Generation.",
                model_name
            ),
        }
    } else {
        // Auto mode → all enabled providers in priority order
        config
            .providers
            .iter()
            .filter(|p| p.enabled && p.api_key.as_ref().map_or(false, |k| !k.is_empty()))
            .collect()
    };

    if candidates.is_empty() {
        anyhow::bail!(
            "No image generation provider configured. Please configure one in Settings > Tool Settings > Image Generation."
        );
    }

    let timeout = config.timeout_seconds;
    let mut failover_log: Vec<String> = Vec::new();
    let mut last_error = String::new();

    for entry in &candidates {
        let impl_ = match super::resolve_provider(&entry.id) {
            Some(i) => i,
            None => {
                failover_log.push(format!("Unknown provider '{}', skipped", entry.id));
                continue;
            }
        };

        let model_name = entry
            .model
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(impl_.default_model());
        let display = impl_.display_name();

        // Validate capabilities before attempting
        let caps = impl_.capabilities();
        if let Err(e) = validate_capabilities(
            &caps,
            display,
            is_edit,
            n,
            aspect_ratio,
            effective_resolution,
            size,
            input_images.len(),
        ) {
            failover_log.push(format!("{}/{} skipped: {}", display, model_name, e));
            app_info!(
                "tool",
                "image_generate",
                "{}/{} skipped (capability mismatch): {}",
                display,
                model_name,
                e
            );
            continue;
        }

        app_info!(
            "tool",
            "image_generate",
            "Image generate [{}/{}]: prompt='{}', size={}, n={}, edit={}, aspectRatio={:?}, resolution={:?}",
            display,
            model_name,
            if prompt.len() > 80 {
                format!("{}...", crate::truncate_utf8(prompt, 80))
            } else {
                prompt.to_string()
            },
            size,
            n,
            is_edit,
            aspect_ratio,
            effective_resolution
        );

        // Retry loop: max 1 retry for retryable errors
        let max_retries: u32 = 1;

        for attempt in 0..=max_retries {
            let params = ImageGenParams {
                api_key: entry.api_key.as_deref().unwrap(),
                base_url: entry.base_url.as_deref(),
                model: model_name,
                prompt,
                size,
                n,
                timeout_secs: timeout,
                extra: entry,
                aspect_ratio,
                resolution: effective_resolution,
                input_images: &input_images,
            };

            match impl_.generate(params).await {
                Ok(result) => {
                    return build_success_result(
                        result,
                        display,
                        model_name,
                        size,
                        aspect_ratio,
                        effective_resolution,
                        is_edit,
                        &failover_log,
                    );
                }
                Err(e) => {
                    let reason = crate::failover::classify_error(&e.to_string());
                    let reason_label = format!("{:?}", reason);

                    if reason.is_retryable() && attempt < max_retries {
                        let delay = crate::failover::retry_delay_ms(attempt, 2000, 10000);
                        failover_log.push(format!(
                            "{}/{} failed ({}), retrying in {}ms...",
                            display, model_name, reason_label, delay
                        ));
                        app_warn!(
                            "tool",
                            "image_generate",
                            "{}/{} failed ({}), retrying in {}ms",
                            display,
                            model_name,
                            reason_label,
                            delay
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        continue;
                    }

                    let err_string = e.to_string();
                    let err_preview = crate::truncate_utf8(&err_string, 200);
                    failover_log.push(format!(
                        "{}/{} failed ({}): {}",
                        display, model_name, reason_label, err_preview
                    ));
                    last_error = e.to_string();
                    app_warn!(
                        "tool",
                        "image_generate",
                        "{}/{} failed ({}): {}",
                        display,
                        model_name,
                        reason_label,
                        err_preview
                    );
                    break; // → next candidate
                }
            }
        }
    }

    // All providers failed
    let log_summary = failover_log.join("\n");
    anyhow::bail!(
        "All image generation providers failed.\n{}\nLast error: {}",
        log_summary,
        crate::truncate_utf8(&last_error, 300)
    )
}
