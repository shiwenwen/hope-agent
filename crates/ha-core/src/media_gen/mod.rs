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

//! 阶段 4 起为 kernel 配置面 facade：provider/链 CRUD（`crud`，
//! mutate_config contract）、候选解析（`resolve`，纯配置函数）与 wire
//! 类型再导出留 kernel；执行机器（adapters / execute / input / probe /
//! catalog / overview / voices）与 `image_generate`·`audio_generate` 两
//! 工具在特征 crate `ha-media`（ha-design 的图/音 artifact 表单直接依赖
//! ha-media——「图/音生成必走 execute_image/execute_audio」红线的入口随
//! 机器在特征侧，仍是唯一入口）。

pub mod crud;
pub mod resolve;
pub mod types;
pub use resolve::{
    resolve_candidates, validate_image_request, ImageRequestSpec, ResolvedCandidate, CONFIG_HINT,
};
pub use types::{
    AudioGenDefaults, AudioKind, AudioModelCaps, ImageEditCaps, ImageGenDefaults, ImageModelCaps,
    MediaDefaultChains, MediaFunction, MediaGenConfig, MediaModality, MediaModelChain,
    MediaModelConfig, MediaModelRef, MediaProviderConfig, MediaVendorKind, AUDIO_DURATIONS_SEC,
    MAX_INPUT_IMAGES, TIMEOUT_CLAMP_SECS, VALID_ASPECT_RATIOS, VALID_RESOLUTIONS,
};
