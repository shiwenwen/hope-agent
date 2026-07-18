use std::collections::HashSet;

use anyhow::Result;
use serde_json::Value;

use super::output::*;
use crate::media_gen::adapters::InputImage;
use crate::media_gen::{
    execute_image, infer_resolution, load_input_image, ImageRequest, UsageMeta, MAX_INPUT_IMAGES,
    VALID_ASPECT_RATIOS, VALID_RESOLUTIONS,
};
use crate::tools::execution::ToolExecContext;

// ── Tool Entry Point ────────────────────────────────────────────

pub(crate) async fn tool_image_generate(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let config = crate::config::cached_config().media_gen.clone();

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
        .filter(|s| !s.is_empty());

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

    // Auto-infer resolution from input images when editing and the caller
    // didn't pin size or resolution explicitly.
    let effective_resolution = if resolution.is_some() {
        resolution
    } else if is_edit && size.is_none() {
        Some(infer_resolution(&input_images))
    } else {
        None
    };

    let outcome = execute_image(
        &config,
        ImageRequest {
            prompt,
            size,
            n,
            aspect_ratio,
            resolution: effective_resolution,
            input_images: &input_images,
            mask: None,
            explicit_model: model_override,
        },
        UsageMeta {
            operation: "tool.image_generate",
            source: "tool",
            session_id: ctx.session_id.clone(),
            agent_id: ctx.agent_id.clone(),
        },
    )
    .await?;

    let effective_size = size.unwrap_or(&config.image_defaults.default_size);
    build_success_result(
        outcome.result,
        &outcome.provider_name,
        &outcome.model_id,
        effective_size,
        aspect_ratio,
        effective_resolution,
        is_edit,
        &outcome.failover_log,
        ctx.session_id.as_deref(),
    )
}
