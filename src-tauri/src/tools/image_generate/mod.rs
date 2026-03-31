use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use base64::Engine;
use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::provider;

pub(crate) mod fal;
pub(crate) mod google;
pub(crate) mod minimax;
pub(crate) mod openai;
pub(crate) mod siliconflow;
pub(crate) mod tongyi;
pub(crate) mod zhipu;

// ── Capabilities System ─────────────────────────────────────────

/// Describes what a provider can do in generate mode.
pub(crate) struct ImageGenModeCapabilities {
    pub max_count: u32,
    pub supports_size: bool,
    pub supports_aspect_ratio: bool,
    pub supports_resolution: bool,
}

/// Describes what a provider can do in edit mode (with input/reference images).
pub(crate) struct ImageGenEditCapabilities {
    pub enabled: bool,
    pub max_count: u32,
    pub max_input_images: u32,
    pub supports_size: bool,
    pub supports_aspect_ratio: bool,
    pub supports_resolution: bool,
}

/// Available geometry options for a provider.
pub(crate) struct ImageGenGeometry {
    pub sizes: Vec<&'static str>,
    pub aspect_ratios: Vec<&'static str>,
    pub resolutions: Vec<&'static str>,
}

/// Full capabilities declaration for a provider.
pub(crate) struct ImageGenCapabilities {
    pub generate: ImageGenModeCapabilities,
    pub edit: ImageGenEditCapabilities,
    pub geometry: Option<ImageGenGeometry>,
}

// ── Input Image (for editing) ───────────────────────────────────

/// A loaded input/reference image ready for provider consumption.
pub(crate) struct InputImage {
    pub data: Vec<u8>,
    pub mime: String,
}

// ── Provider Trait ──────────────────────────────────────────────

/// Unified parameters for image generation (provider differences are handled internally).
pub(crate) struct ImageGenParams<'a> {
    pub api_key: &'a str,
    pub base_url: Option<&'a str>,
    pub model: &'a str,
    pub prompt: &'a str,
    pub size: &'a str,
    pub n: u32,
    pub timeout_secs: u64,
    /// Provider-specific extra fields (e.g. thinking_level for Google)
    pub extra: &'a ImageGenProviderEntry,
    /// Aspect ratio hint (e.g. "1:1", "16:9", "9:16")
    pub aspect_ratio: Option<&'a str>,
    /// Resolution hint: "1K", "2K", or "4K"
    pub resolution: Option<&'a str>,
    /// Reference/input images for editing
    pub input_images: &'a [InputImage],
}

/// Trait for image generation providers.
pub(crate) trait ImageGenProviderImpl: Send + Sync {
    /// Unique provider id (lowercase), e.g. "openai", "google", "fal", "minimax"
    #[allow(dead_code)]
    fn id(&self) -> &str;

    /// Human-readable display name, e.g. "OpenAI", "Google", "Fal", "MiniMax"
    fn display_name(&self) -> &str;

    /// Default model when user hasn't configured one
    fn default_model(&self) -> &str;

    /// Declare provider capabilities (generate/edit/geometry)
    fn capabilities(&self) -> ImageGenCapabilities;

    /// Execute image generation
    fn generate<'a>(
        &'a self,
        params: ImageGenParams<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<ImageGenResult>> + Send + 'a>>;
}

/// Resolve a provider implementation by id string.
pub fn resolve_provider(id: &str) -> Option<Box<dyn ImageGenProviderImpl>> {
    match id.to_lowercase().as_str() {
        "openai" => Some(Box::new(openai::OpenAIProvider)),
        "google" => Some(Box::new(google::GoogleProvider)),
        "fal" => Some(Box::new(fal::FalProvider)),
        "minimax" => Some(Box::new(minimax::MiniMaxProvider)),
        "siliconflow" => Some(Box::new(siliconflow::SiliconFlowProvider)),
        "zhipu" => Some(Box::new(zhipu::ZhipuProvider)),
        "tongyi" => Some(Box::new(tongyi::TongyiProvider)),
        _ => None,
    }
}

/// Known built-in provider ids.
pub fn known_provider_ids() -> &'static [&'static str] {
    &[
        "openai",
        "google",
        "fal",
        "minimax",
        "siliconflow",
        "zhipu",
        "tongyi",
    ]
}

/// Normalize provider id for backward compatibility (e.g. "OpenAI" → "openai").
fn normalize_provider_id(id: &str) -> String {
    match id {
        "OpenAI" => "openai".to_string(),
        "Google" => "google".to_string(),
        "Fal" => "fal".to_string(),
        "MiniMax" | "Minimax" => "minimax".to_string(),
        "SiliconFlow" | "Siliconflow" => "siliconflow".to_string(),
        "Zhipu" | "ZhipuAI" | "zhipuai" => "zhipu".to_string(),
        "Tongyi" | "TongyiWanxiang" | "DashScope" => "tongyi".to_string(),
        other => other.to_lowercase(),
    }
}

// ── Image Generation Provider Config ────────────────────────────

/// A single image generation provider entry with credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenProviderEntry {
    pub id: String,
    pub enabled: bool,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// Google-specific: thinking level ("MINIMAL" or "HIGH"), default "MINIMAL"
    #[serde(default)]
    pub thinking_level: Option<String>,
}

impl Default for ImageGenProviderEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            enabled: false,
            api_key: None,
            base_url: None,
            model: None,
            thinking_level: None,
        }
    }
}

/// Persistent image generation configuration, stored in config.json
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenConfig {
    /// Ordered list of providers (order = priority). First enabled provider with API key is used.
    #[serde(default = "default_providers")]
    pub providers: Vec<ImageGenProviderEntry>,
    /// Request timeout in seconds (default 60)
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    /// Default image size (default "1024x1024")
    #[serde(default = "default_size")]
    pub default_size: String,
}

fn default_providers() -> Vec<ImageGenProviderEntry> {
    vec![
        ImageGenProviderEntry {
            id: "openai".to_string(),
            ..Default::default()
        },
        ImageGenProviderEntry {
            id: "google".to_string(),
            ..Default::default()
        },
        ImageGenProviderEntry {
            id: "fal".to_string(),
            ..Default::default()
        },
        ImageGenProviderEntry {
            id: "minimax".to_string(),
            ..Default::default()
        },
        ImageGenProviderEntry {
            id: "siliconflow".to_string(),
            ..Default::default()
        },
        ImageGenProviderEntry {
            id: "zhipu".to_string(),
            ..Default::default()
        },
        ImageGenProviderEntry {
            id: "tongyi".to_string(),
            ..Default::default()
        },
    ]
}

fn default_timeout() -> u64 {
    60
}

fn default_size() -> String {
    "1024x1024".to_string()
}

impl Default for ImageGenConfig {
    fn default() -> Self {
        Self {
            providers: default_providers(),
            timeout_seconds: default_timeout(),
            default_size: default_size(),
        }
    }
}

/// Ensure all known providers exist in the config and normalize ids.
pub fn backfill_providers(config: &mut ImageGenConfig) {
    // Normalize existing ids (backward compat: "OpenAI" → "openai")
    for p in &mut config.providers {
        p.id = normalize_provider_id(&p.id);
    }
    // Ensure all known providers exist
    for id in known_provider_ids() {
        if !config.providers.iter().any(|p| p.id == *id) {
            config.providers.push(ImageGenProviderEntry {
                id: id.to_string(),
                ..Default::default()
            });
        }
    }
}

// ── Generated Image ─────────────────────────────────────────────

pub(crate) struct GeneratedImage {
    pub data: Vec<u8>,
    pub mime: String,
    pub revised_prompt: Option<String>,
}

/// Result from image generation, containing images and optional accompanying text.
pub(crate) struct ImageGenResult {
    pub images: Vec<GeneratedImage>,
    /// Accompanying text content from the model (e.g. Gemini returns text alongside images).
    pub text: Option<String>,
}

// ── Aspect Ratio / Resolution Constants ─────────────────────────

const VALID_ASPECT_RATIOS: &[&str] = &[
    "1:1", "2:3", "3:2", "3:4", "4:3", "4:5", "5:4", "9:16", "16:9", "21:9",
];

const VALID_RESOLUTIONS: &[&str] = &["1K", "2K", "4K"];

const MAX_INPUT_IMAGES: usize = 5;

// ── Public Helpers ──────────────────────────────────────────────

/// Check if at least one provider is enabled with an API key.
#[allow(dead_code)]
pub fn has_configured_provider() -> bool {
    provider::load_store()
        .map(|s| has_configured_provider_from_config(&s.image_generate))
        .unwrap_or(false)
}

/// Check from a config reference (avoids re-loading store).
pub fn has_configured_provider_from_config(config: &ImageGenConfig) -> bool {
    config
        .providers
        .iter()
        .any(|p| p.enabled && p.api_key.as_ref().map_or(false, |k| !k.is_empty()))
}

/// Get the display name for a provider entry.
pub fn provider_display_name(entry: &ImageGenProviderEntry) -> String {
    resolve_provider(&entry.id)
        .map(|p| p.display_name().to_string())
        .unwrap_or_else(|| entry.id.clone())
}

/// Get the effective model name for a provider entry.
pub fn effective_model(entry: &ImageGenProviderEntry) -> String {
    entry
        .model
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            resolve_provider(&entry.id)
                .map(|p| p.default_model().to_string())
                .unwrap_or_else(|| "unknown".to_string())
        })
}

/// Find a provider entry by model name (for LLM tool `model` parameter routing).
fn find_provider_by_model<'a>(
    model: &str,
    config: &'a ImageGenConfig,
) -> Option<&'a ImageGenProviderEntry> {
    let enabled_providers = config
        .providers
        .iter()
        .filter(|p| p.enabled && p.api_key.as_ref().map_or(false, |k| !k.is_empty()));

    // 1. Exact match on user-configured model
    for entry in config
        .providers
        .iter()
        .filter(|p| p.enabled && p.api_key.as_ref().map_or(false, |k| !k.is_empty()))
    {
        if entry.model.as_deref() == Some(model) {
            return Some(entry);
        }
    }

    // 2. Match against provider's default model
    for entry in enabled_providers {
        if let Some(impl_) = resolve_provider(&entry.id) {
            if impl_.default_model() == model {
                return Some(entry);
            }
        }
    }

    None
}

// ── Input Image Loading ─────────────────────────────────────────

/// Load an input image from a local file path or HTTP(S) URL.
async fn load_input_image(path_or_url: &str) -> Result<InputImage> {
    let trimmed = path_or_url.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Empty image path/URL");
    }

    // Data URL
    if trimmed.starts_with("data:") {
        return decode_data_url(trimmed);
    }

    // HTTP(S) URL
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        let resp = client.get(trimmed).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!(
                "Failed to download image from {} ({})",
                trimmed,
                resp.status()
            );
        }
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("image/png")
            .to_string();
        let mime = content_type
            .split(';')
            .next()
            .unwrap_or("image/png")
            .trim()
            .to_string();
        let data = resp.bytes().await?.to_vec();
        return Ok(InputImage { data, mime });
    }

    // Local file path (expand ~ to home dir)
    let resolved = if trimmed.starts_with("~/") || trimmed.starts_with("~\\") {
        if let Some(home) = dirs::home_dir() {
            home.join(&trimmed[2..])
        } else {
            std::path::PathBuf::from(trimmed)
        }
    } else if trimmed.starts_with("file://") {
        std::path::PathBuf::from(&trimmed[7..])
    } else {
        std::path::PathBuf::from(trimmed)
    };

    let data = tokio::fs::read(&resolved).await.map_err(|e| {
        anyhow::anyhow!("Failed to read image file '{}': {}", resolved.display(), e)
    })?;

    // Infer MIME from extension
    let mime = match resolved.extension().and_then(|e| e.to_str()) {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        _ => "image/png",
    };

    Ok(InputImage {
        data,
        mime: mime.to_string(),
    })
}

/// Decode a data URL into InputImage.
fn decode_data_url(url: &str) -> Result<InputImage> {
    // data:image/png;base64,xxxx
    let after_data = url.strip_prefix("data:").unwrap_or(url);
    let (header, b64) = after_data
        .split_once(',')
        .ok_or_else(|| anyhow::anyhow!("Invalid data URL format"))?;
    let mime = header.split(';').next().unwrap_or("image/png").to_string();
    let data = base64::engine::general_purpose::STANDARD.decode(b64.trim())?;
    Ok(InputImage { data, mime })
}

/// Infer resolution from input images using the `image` crate.
fn infer_resolution(images: &[InputImage]) -> &'static str {
    let mut max_dim: u32 = 0;
    for img in images {
        if let Ok(reader) =
            image::ImageReader::new(std::io::Cursor::new(&img.data)).with_guessed_format()
        {
            if let Ok(dims) = reader.into_dimensions() {
                max_dim = max_dim.max(dims.0).max(dims.1);
            }
        }
    }
    if max_dim >= 3000 {
        "4K"
    } else if max_dim >= 1500 {
        "2K"
    } else {
        "1K"
    }
}

/// Validate tool parameters against provider capabilities.
fn validate_capabilities(
    caps: &ImageGenCapabilities,
    provider_name: &str,
    is_edit: bool,
    count: u32,
    aspect_ratio: Option<&str>,
    resolution: Option<&str>,
    size: &str,
    input_count: usize,
) -> Result<()> {
    let mode_caps = if is_edit {
        &caps.edit_as_mode()
    } else {
        &caps.generate
    };

    if is_edit {
        if !caps.edit.enabled {
            anyhow::bail!(
                "{} does not support reference-image editing.",
                provider_name
            );
        }
        if input_count as u32 > caps.edit.max_input_images {
            anyhow::bail!(
                "{} edit supports at most {} reference image(s), got {}.",
                provider_name,
                caps.edit.max_input_images,
                input_count
            );
        }
    }

    let max_count = if is_edit {
        caps.edit.max_count
    } else {
        mode_caps.max_count
    };
    if count > max_count {
        anyhow::bail!(
            "{} {} supports at most {} image(s), requested {}.",
            provider_name,
            if is_edit { "edit" } else { "generate" },
            max_count,
            count
        );
    }

    if aspect_ratio.is_some() && !mode_caps.supports_aspect_ratio {
        anyhow::bail!(
            "{} {} does not support aspectRatio.",
            provider_name,
            if is_edit { "edit" } else { "generate" }
        );
    }

    if let Some(ar) = aspect_ratio {
        if let Some(ref geo) = caps.geometry {
            if !geo.aspect_ratios.is_empty() && !geo.aspect_ratios.contains(&ar) {
                anyhow::bail!(
                    "{} aspectRatio must be one of: {}",
                    provider_name,
                    geo.aspect_ratios.join(", ")
                );
            }
        }
    }

    if resolution.is_some() && !mode_caps.supports_resolution {
        anyhow::bail!(
            "{} {} does not support resolution.",
            provider_name,
            if is_edit { "edit" } else { "generate" }
        );
    }

    if let Some(res) = resolution {
        if let Some(ref geo) = caps.geometry {
            if !geo.resolutions.is_empty() && !geo.resolutions.contains(&res) {
                anyhow::bail!(
                    "{} resolution must be one of: {}",
                    provider_name,
                    geo.resolutions.join(", ")
                );
            }
        }
    }

    if size != "1024x1024" && !mode_caps.supports_size {
        // Only validate non-default sizes
        anyhow::bail!(
            "{} {} does not support custom size.",
            provider_name,
            if is_edit { "edit" } else { "generate" }
        );
    }

    if mode_caps.supports_size {
        if let Some(ref geo) = caps.geometry {
            if !geo.sizes.is_empty() && !geo.sizes.contains(&size) {
                anyhow::bail!(
                    "{} size must be one of: {}",
                    provider_name,
                    geo.sizes.join(", ")
                );
            }
        }
    }

    Ok(())
}

impl ImageGenCapabilities {
    /// Get mode capabilities for edit as ImageGenModeCapabilities reference.
    fn edit_as_mode(&self) -> ImageGenModeCapabilities {
        ImageGenModeCapabilities {
            max_count: self.edit.max_count,
            supports_size: self.edit.supports_size,
            supports_aspect_ratio: self.edit.supports_aspect_ratio,
            supports_resolution: self.edit.supports_resolution,
        }
    }
}

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
        let impl_ = match resolve_provider(&entry.id) {
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

// ── List Action ─────────────────────────────────────────────────

/// Build formatted text listing all available providers and their capabilities.
fn build_list_result(config: &ImageGenConfig) -> Result<String> {
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
        let impl_ = match resolve_provider(&entry.id) {
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
fn build_success_result(
    gen_result: ImageGenResult,
    display_name: &str,
    model: &str,
    size: &str,
    aspect_ratio: Option<&str>,
    resolution: Option<&str>,
    is_edit: bool,
    failover_log: &[String],
) -> Result<String> {
    let images = gen_result.images;
    let accompanying_text = gen_result.text;

    // Save images to disk
    let save_dir = crate::paths::generated_images_dir()?;
    std::fs::create_dir_all(&save_dir)?;
    let timestamp = Local::now().format("%Y%m%d_%H%M%S");
    let mut saved_paths = Vec::new();

    for (i, img) in images.iter().enumerate() {
        let ext = if img.mime.contains("jpeg") || img.mime.contains("jpg") {
            "jpg"
        } else {
            "png"
        };
        let filename = format!("{}_{}.{}", timestamp, i, ext);
        let path = save_dir.join(&filename);
        match std::fs::write(&path, &img.data) {
            Ok(_) => saved_paths.push(path.to_string_lossy().to_string()),
            Err(e) => {
                app_warn!(
                    "tool",
                    "image_generate",
                    "Failed to save generated image: {}",
                    e
                );
            }
        }
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

    for path in &saved_paths {
        text_parts.push(format!("Saved to: {}", path));
    }
    if !images.is_empty() {
        if let Some(ref rp) = images[0].revised_prompt {
            text_parts.push(format!("Revised prompt: {}", rp));
        }
    }
    if let Some(ref text) = accompanying_text {
        text_parts.push(format!("Model response: {}", text));
    }

    // Embed media URLs so the event system can extract them for frontend display
    let media_urls_json = serde_json::to_string(&saved_paths).unwrap_or_default();
    let result = format!(
        "__MEDIA_URLS__{}\n{}",
        media_urls_json,
        text_parts.join("\n")
    );

    // Log detailed completion info
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
