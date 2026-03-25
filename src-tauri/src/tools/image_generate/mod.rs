use anyhow::Result;
use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::provider;

mod openai;
mod google;
mod fal;

// ── Image Generation Provider Config ────────────────────────────

/// Supported image generation providers
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ImageGenProvider {
    /// OpenAI DALL-E / gpt-image-1
    OpenAI,
    /// Google Gemini image generation
    Google,
    /// Fal (Flux) image generation
    Fal,
}

impl std::fmt::Display for ImageGenProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenAI => write!(f, "OpenAI"),
            Self::Google => write!(f, "Google"),
            Self::Fal => write!(f, "Fal"),
        }
    }
}

/// A single image generation provider entry with credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenProviderEntry {
    pub id: ImageGenProvider,
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

/// Persistent image generation configuration, stored in config.json
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenConfig {
    /// Ordered list of providers. First enabled provider with API key is used.
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
            id: ImageGenProvider::OpenAI,
            enabled: false,
            api_key: None,
            base_url: None,
            model: None,
            thinking_level: None,
        },
        ImageGenProviderEntry {
            id: ImageGenProvider::Google,
            enabled: false,
            api_key: None,
            base_url: None,
            model: None,
            thinking_level: None,
        },
        ImageGenProviderEntry {
            id: ImageGenProvider::Fal,
            enabled: false,
            api_key: None,
            base_url: None,
            model: None,
            thinking_level: None,
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

/// Ensure all known providers exist in the config (for forward compatibility).
pub fn backfill_providers(config: &mut ImageGenConfig) {
    let known = [
        ImageGenProvider::OpenAI,
        ImageGenProvider::Google,
        ImageGenProvider::Fal,
    ];
    for id in &known {
        if !config.providers.iter().any(|p| &p.id == id) {
            config.providers.push(ImageGenProviderEntry {
                id: id.clone(),
                enabled: false,
                api_key: None,
                base_url: None,
                model: None,
                thinking_level: None,
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

// ── Tool Entry Point ────────────────────────────────────────────

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

    let provider_override = args
        .get("provider")
        .and_then(|v| v.as_str());

    // Resolve provider: explicit override or first enabled with API key
    let entry = if let Some(pid) = provider_override {
        let pid_lower = pid.to_lowercase();
        config
            .providers
            .iter()
            .find(|p| {
                let id_str = format!("{:?}", p.id).to_lowercase();
                id_str == pid_lower || format!("{}", p.id).to_lowercase() == pid_lower
            })
            .filter(|p| p.api_key.as_ref().map_or(false, |k| !k.is_empty()))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Provider '{}' not found or missing API key. Configure it in Settings > Tool Settings > Image Generation.",
                    pid
                )
            })?
    } else {
        config
            .providers
            .iter()
            .find(|p| p.enabled && p.api_key.as_ref().map_or(false, |k| !k.is_empty()))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No image generation provider configured. Please configure one in Settings > Tool Settings > Image Generation."
                )
            })?
    };

    let api_key = entry.api_key.as_deref().unwrap();
    let base_url = entry.base_url.as_deref();
    let model = entry.model.as_deref();
    let timeout = config.timeout_seconds;

    app_info!(
        "tool",
        "image_generate",
        "Image generate [{}]: prompt='{}', size={}, n={}",
        entry.id,
        if prompt.len() > 80 {
            format!("{}...", crate::truncate_utf8(prompt, 80))
        } else {
            prompt.to_string()
        },
        size,
        n
    );

    let images = match entry.id {
        ImageGenProvider::OpenAI => {
            openai::generate(api_key, base_url, model, prompt, size, n, timeout).await?
        }
        ImageGenProvider::Google => {
            let thinking_level = entry.thinking_level.as_deref();
            google::generate(api_key, base_url, model, prompt, thinking_level, timeout).await?
        }
        ImageGenProvider::Fal => {
            fal::generate(api_key, base_url, model, prompt, size, n, timeout).await?
        }
    };

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

    // Build result string with __MEDIA_URLS__ prefix for frontend image display
    let mut text_parts = Vec::new();
    text_parts.push(format!(
        "Generated {} image{} with {}/{}.",
        images.len(),
        if images.len() > 1 { "s" } else { "" },
        entry.id,
        model.unwrap_or("default")
    ));
    text_parts.push(format!("Size: {}", size));
    for path in &saved_paths {
        text_parts.push(format!("Saved to: {}", path));
    }
    if let Some(ref rp) = images[0].revised_prompt {
        text_parts.push(format!("Revised prompt: {}", rp));
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
        "Image generation complete: {} image(s), {} saved",
        images.len(),
        saved_paths.len()
    );

    Ok(result)
}
