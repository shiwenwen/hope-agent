use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use serde::{Deserialize, Serialize};

// ── Capabilities System ─────────────────────────────────────────

/// Describes what a provider can do in generate mode.
pub struct ImageGenModeCapabilities {
    pub max_count: u32,
    pub supports_size: bool,
    pub supports_aspect_ratio: bool,
    pub supports_resolution: bool,
}

/// Describes what a provider can do in edit mode (with input/reference images).
pub struct ImageGenEditCapabilities {
    pub enabled: bool,
    pub max_count: u32,
    pub max_input_images: u32,
    pub supports_size: bool,
    pub supports_aspect_ratio: bool,
    pub supports_resolution: bool,
}

/// Available geometry options for a provider.
pub struct ImageGenGeometry {
    pub sizes: Vec<&'static str>,
    pub aspect_ratios: Vec<&'static str>,
    pub resolutions: Vec<&'static str>,
}

/// Full capabilities declaration for a provider.
pub struct ImageGenCapabilities {
    pub generate: ImageGenModeCapabilities,
    pub edit: ImageGenEditCapabilities,
    pub geometry: Option<ImageGenGeometry>,
}

impl ImageGenCapabilities {
    /// Get mode capabilities for edit as ImageGenModeCapabilities reference.
    pub(super) fn edit_as_mode(&self) -> ImageGenModeCapabilities {
        ImageGenModeCapabilities {
            max_count: self.edit.max_count,
            supports_size: self.edit.supports_size,
            supports_aspect_ratio: self.edit.supports_aspect_ratio,
            supports_resolution: self.edit.supports_resolution,
        }
    }
}

// ── Input Image (for editing) ───────────────────────────────────

/// A loaded input/reference image ready for provider consumption.
pub struct InputImage {
    pub data: Vec<u8>,
    pub mime: String,
}

// ── Provider Trait ──────────────────────────────────────────────

/// Unified parameters for image generation (provider differences are handled internally).
pub struct ImageGenParams<'a> {
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
pub trait ImageGenProviderImpl: Send + Sync {
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

// ── Generated Image ─────────────────────────────────────────────

pub struct GeneratedImage {
    pub data: Vec<u8>,
    pub mime: String,
    pub revised_prompt: Option<String>,
}

/// Result from image generation, containing images and optional accompanying text.
pub struct ImageGenResult {
    pub images: Vec<GeneratedImage>,
    /// Accompanying text content from the model (e.g. Gemini returns text alongside images).
    pub text: Option<String>,
}

// ── Image Generation Provider Config ────────────────────────────

/// A single image generation provider entry with credentials.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

// ── Aspect Ratio / Resolution Constants ─────────────────────────

pub(super) const VALID_ASPECT_RATIOS: &[&str] = &[
    "1:1", "2:3", "3:2", "3:4", "4:3", "4:5", "5:4", "9:16", "16:9", "21:9",
];

pub(super) const VALID_RESOLUTIONS: &[&str] = &["1K", "2K", "4K"];

pub(super) const MAX_INPUT_IMAGES: usize = 5;

/// Ensure all known providers exist in the config and normalize ids.
pub fn backfill_providers(config: &mut ImageGenConfig) {
    // Normalize existing ids (backward compat: "OpenAI" → "openai")
    for p in &mut config.providers {
        p.id = super::normalize_provider_id(&p.id);
    }
    // Ensure all known providers exist
    for id in super::known_provider_ids() {
        if !config.providers.iter().any(|p| p.id == *id) {
            config.providers.push(ImageGenProviderEntry {
                id: id.to_string(),
                ..Default::default()
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backfill_adds_missing_known_providers_to_empty_list() {
        let mut cfg = ImageGenConfig {
            providers: Vec::new(),
            timeout_seconds: 60,
            default_size: "1024x1024".to_string(),
        };
        backfill_providers(&mut cfg);
        for id in super::super::known_provider_ids() {
            assert!(
                cfg.providers.iter().any(|p| &p.id == id),
                "missing provider {}",
                id
            );
        }
    }

    #[test]
    fn backfill_normalizes_legacy_display_names() {
        let mut cfg = ImageGenConfig {
            providers: vec![
                ImageGenProviderEntry {
                    id: "OpenAI".to_string(),
                    enabled: true,
                    api_key: Some("sk".to_string()),
                    ..Default::default()
                },
                ImageGenProviderEntry {
                    id: "Minimax".to_string(),
                    ..Default::default()
                },
            ],
            timeout_seconds: 60,
            default_size: "1024x1024".to_string(),
        };
        backfill_providers(&mut cfg);
        assert!(cfg.providers.iter().any(|p| p.id == "openai" && p.enabled));
        assert!(cfg.providers.iter().any(|p| p.id == "minimax"));
        // Legacy ids must have been replaced, not duplicated.
        assert!(!cfg.providers.iter().any(|p| p.id == "OpenAI"));
        assert!(!cfg.providers.iter().any(|p| p.id == "Minimax"));
    }

    #[test]
    fn backfill_preserves_existing_config_for_known_provider() {
        let mut cfg = ImageGenConfig {
            providers: vec![ImageGenProviderEntry {
                id: "openai".to_string(),
                enabled: true,
                api_key: Some("kept".to_string()),
                model: Some("custom-model".to_string()),
                ..Default::default()
            }],
            timeout_seconds: 60,
            default_size: "1024x1024".to_string(),
        };
        backfill_providers(&mut cfg);
        let openai = cfg.providers.iter().find(|p| p.id == "openai").unwrap();
        assert_eq!(openai.api_key.as_deref(), Some("kept"));
        assert_eq!(openai.model.as_deref(), Some("custom-model"));
        assert!(openai.enabled);
    }
}
