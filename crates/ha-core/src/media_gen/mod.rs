//! Unified media-generation subsystem (image / audio; `video` reserved).
//!
//! "Provider → models → per-function default chain", mirroring the STT
//! subsystem: user-managed provider list (one credentials entry, many
//! models), data-driven per-model capabilities, and per-function default
//! chains (image / speech / music / sfx) with an auto fallback over
//! provider order. Consumed by the `image_generate` / `audio_generate`
//! chat tools and the design space's image / audio artifact forms — all of
//! them resolve candidates and execute through this module so failover,
//! capability validation, and usage accounting live in exactly one place.
//!
//! See `docs/architecture/media-generation.md`.

pub mod adapters;
mod catalog;
pub mod crud;
mod execute;
mod input;
mod overview;
pub mod probe;
mod resolve;
mod types;
pub mod voices;

pub use catalog::{media_provider_templates, MediaProviderTemplate, OPENAI_TTS_VOICES};
pub use execute::{
    execute_audio, execute_image, AudioRequest, ImageRequest, MediaExecOutcome, UsageMeta,
};
pub use input::{decode_data_url, infer_resolution, load_input_image, load_input_images};
pub use overview::{
    media_gen_overview, MediaCandidateOverview, MediaFunctionOverview, MediaGenOverview,
};
pub use resolve::{
    resolve_candidates, validate_image_request, ImageRequestSpec, ResolvedCandidate, CONFIG_HINT,
};
pub use types::{
    AudioGenDefaults, AudioKind, AudioModelCaps, ImageEditCaps, ImageGenDefaults, ImageModelCaps,
    MediaDefaultChains, MediaFunction, MediaGenConfig, MediaModality, MediaModelChain,
    MediaModelConfig, MediaModelRef, MediaProviderConfig, MediaVendorKind, AUDIO_DURATIONS_SEC,
    MAX_INPUT_IMAGES, TIMEOUT_CLAMP_SECS, VALID_ASPECT_RATIOS, VALID_RESOLUTIONS,
};
