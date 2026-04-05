// ── Sub-modules: provider implementations ──────────────────────
pub(crate) mod fal;
pub(crate) mod google;
pub(crate) mod minimax;
pub(crate) mod openai;
pub(crate) mod siliconflow;
pub(crate) mod tongyi;
pub(crate) mod zhipu;

// ── Sub-modules: split from this file ──────────────────────────
mod generate;
mod helpers;
mod output;
mod types;

// ── Re-exports ─────────────────────────────────────────────────
// Types (used by provider implementations and external callers)
pub use types::{
    GeneratedImage, ImageGenCapabilities, ImageGenEditCapabilities, ImageGenGeometry,
    ImageGenModeCapabilities, ImageGenParams, ImageGenProviderImpl, ImageGenResult, InputImage,
};
// Config types (used by provider.rs, commands, chat_engine, etc.)
pub use types::{ImageGenConfig, ImageGenProviderEntry};
// Config helpers
pub use types::backfill_providers;
// Public helpers
pub use helpers::{
    effective_model, has_configured_provider, has_configured_provider_from_config,
    provider_display_name,
};
// Tool entry point
pub(crate) use generate::tool_image_generate;

// ── Small routing functions kept in mod.rs ─────────────────────

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
