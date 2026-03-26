use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::provider;

pub(crate) mod openai;
pub(crate) mod google;
pub(crate) mod fal;

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
}

/// Trait for image generation providers.
pub(crate) trait ImageGenProviderImpl: Send + Sync {
    /// Unique provider id (lowercase), e.g. "openai", "google", "fal"
    fn id(&self) -> &str;

    /// Human-readable display name, e.g. "OpenAI", "Google", "Fal"
    fn display_name(&self) -> &str;

    /// Default model when user hasn't configured one
    fn default_model(&self) -> &str;

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
        _ => None,
    }
}

/// Known built-in provider ids.
pub fn known_provider_ids() -> &'static [&'static str] {
    &["openai", "google", "fal"]
}

/// Normalize provider id for backward compatibility (e.g. "OpenAI" → "openai").
fn normalize_provider_id(id: &str) -> String {
    match id {
        "OpenAI" => "openai".to_string(),
        "Google" => "google".to_string(),
        "Fal" => "fal".to_string(),
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
        ImageGenProviderEntry { id: "openai".to_string(), ..Default::default() },
        ImageGenProviderEntry { id: "google".to_string(), ..Default::default() },
        ImageGenProviderEntry { id: "fal".to_string(), ..Default::default() },
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
    config.providers.iter().any(|p| {
        p.enabled && p.api_key.as_ref().map_or(false, |k| !k.is_empty())
    })
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
    let enabled_providers = config.providers.iter().filter(|p| {
        p.enabled && p.api_key.as_ref().map_or(false, |k| !k.is_empty())
    });

    // 1. Exact match on user-configured model
    for entry in config.providers.iter().filter(|p| {
        p.enabled && p.api_key.as_ref().map_or(false, |k| !k.is_empty())
    }) {
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

// ── Tool Entry Point (with Failover) ────────────────────────────

pub(crate) async fn tool_image_generate(args: &Value) -> Result<String> {
    let config = provider::load_store()
        .map(|s| s.image_generate)
        .unwrap_or_default();

    let prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'prompt' parameter"))?;

    let size = args
        .get("size")
        .and_then(|v| v.as_str())
        .unwrap_or(&config.default_size);

    let n = args
        .get("n")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .min(4)
        .max(1) as u32;

    let model_override = args
        .get("model")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty() && *s != "auto");

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

        app_info!(
            "tool",
            "image_generate",
            "Image generate [{}/{}]: prompt='{}', size={}, n={}",
            display,
            model_name,
            if prompt.len() > 80 {
                format!("{}...", crate::truncate_utf8(prompt, 80))
            } else {
                prompt.to_string()
            },
            size,
            n
        );

        // Retry loop: max 1 retry for retryable errors
        let max_retries: u32 = 1;
        let mut succeeded = false;

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
            };

            match impl_.generate(params).await {
                Ok(result) => {
                    return build_success_result(
                        result, display, model_name, size, &failover_log,
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

/// Build the success result string with failover transparency.
fn build_success_result(
    gen_result: ImageGenResult,
    display_name: &str,
    model: &str,
    size: &str,
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
    text_parts.push(format!(
        "Generated {} image{} with {}/{}.",
        images.len(),
        if images.len() > 1 { "s" } else { "" },
        display_name,
        model
    ));
    text_parts.push(format!("Size: {}", size));

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

    app_info!(
        "tool",
        "image_generate",
        "Image generation complete: {} image(s), {} saved, provider={}/{}",
        images.len(),
        saved_paths.len(),
        display_name,
        model
    );

    Ok(result)
}
